//! 单写入口 `DbWriter`（17 第 2.9 节，ADR 0020）：Hub 的所有写库都经过这里。
//!
//! - 只有一个写连接，用互斥锁串行；读走另一个只读连接（WAL 下读不等写）。
//! - **普通数据**（特征窗口、状态记录、出网日志、提醒记录）用 [`DbWriter::enqueue`] 排队，满 [`BATCH_MAX`] 条或最早一条
//!   排了 [`BATCH_DELAY`] 时在一个事务里批量提交；每条一个保存点，一条失败只丢这一条。
//! - **用户确认类数据**（同意、设置、反馈、自评、对话、日程等）用 [`DbWriter::write_sync`]：先提交排队中的数据（保持先后顺序），
//!   再在调用方线程上执行，返回时已经落盘（NFR-REL-03）。
//!
//! 排队的任务里不要再开事务（批量提交本身在事务里）；`write_sync` 不包事务，里面可以自己开。

use std::ops::Deref;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{Db, StoreError};

/// 排队的写任务满这么多条就提交。
pub const BATCH_MAX: usize = 100;
/// 最早一条排队的写任务最多等这么久就提交。
pub const BATCH_DELAY: Duration = Duration::from_secs(5);

type Job = Box<dyn FnOnce(&Db) -> Result<(), StoreError> + Send>;

struct State {
    db: Db,
    /// `(标签, 任务)`；标签只用于日志，不含用户数据
    pending: Vec<(&'static str, Job)>,
    oldest: Option<Instant>,
}

struct Shared {
    state: Mutex<State>,
    stop: Mutex<bool>,
    wake: Condvar,
    delay: Duration,
}

pub struct DbWriter {
    shared: Arc<Shared>,
    ticker: Option<JoinHandle<()>>,
}

impl DbWriter {
    /// 接管写连接，并起一个定时线程按 [`BATCH_DELAY`] 提交排队的数据。
    pub fn new(db: Db) -> Self {
        Self::with_delay(db, BATCH_DELAY)
    }

    pub fn with_delay(db: Db, delay: Duration) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                db,
                pending: Vec::new(),
                oldest: None,
            }),
            stop: Mutex::new(false),
            wake: Condvar::new(),
            delay,
        });
        let s = shared.clone();
        let ticker = std::thread::Builder::new()
            .name("xq-db-writer".into())
            .spawn(move || tick(&s))
            .expect("创建写库线程失败");
        Self {
            shared,
            ticker: Some(ticker),
        }
    }

    /// 普通数据：排队，稍后批量提交。失败只记日志（带上 `label`），不影响别的任务。
    pub fn enqueue(
        &self,
        label: &'static str,
        job: impl FnOnce(&Db) -> Result<(), StoreError> + Send + 'static,
    ) {
        let mut st = self.shared.lock();
        st.pending.push((label, Box::new(job)));
        st.oldest.get_or_insert_with(Instant::now);
        if st.pending.len() >= BATCH_MAX {
            flush(&mut st);
        }
    }

    /// 用户确认类数据：先提交排队中的数据，再在写连接上执行 `f`，返回时已经落盘。
    pub fn write_sync<T>(&self, f: impl FnOnce(&Db) -> T) -> T {
        f(&self.lock())
    }

    /// 拿到写连接（先提交排队中的数据）。持有期间其他写入都要等，用完立即释放，不要跨 `.await` 持有。
    pub fn lock(&self) -> WriteGuard<'_> {
        let mut st = self.shared.lock();
        flush(&mut st);
        WriteGuard(st)
    }

    /// 立即提交排队中的数据（导出等要读到最新数据的场合）。
    pub fn flush(&self) {
        flush(&mut self.shared.lock());
    }

    /// 排队中的任务数。
    pub fn pending(&self) -> usize {
        self.shared.lock().pending.len()
    }
}

impl Drop for DbWriter {
    /// 退出时提交剩下的数据。
    fn drop(&mut self) {
        *self.shared.stop.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.shared.wake.notify_all();
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
        self.flush();
    }
}

/// [`DbWriter::lock`] 的返回值，可当 `&Db` 用。
pub struct WriteGuard<'a>(MutexGuard<'a, State>);

impl Deref for WriteGuard<'_> {
    type Target = Db;
    fn deref(&self) -> &Db {
        &self.0.db
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // 某个任务 panic 后连接本身仍可用，不让一次 panic 拖垮之后所有写入
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn tick(s: &Shared) {
    let step = (s.delay / 5).clamp(Duration::from_millis(10), Duration::from_secs(1));
    let mut stop = s.stop.lock().unwrap_or_else(|e| e.into_inner());
    while !*stop {
        stop = s
            .wake
            .wait_timeout(stop, step)
            .unwrap_or_else(|e| e.into_inner())
            .0;
        let mut st = s.lock();
        if st.oldest.is_some_and(|t| t.elapsed() >= s.delay) {
            flush(&mut st);
        }
    }
}

/// 在一个事务里执行全部排队的任务，每个任务一个保存点。开不了事务时逐条自动提交。
fn flush(st: &mut State) {
    st.oldest = None;
    if st.pending.is_empty() {
        return;
    }
    let jobs = std::mem::take(&mut st.pending);
    let conn = st.db.conn();
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => Some(tx),
        Err(e) => {
            eprintln!("批量写库开不了事务，逐条写入：{e}");
            None
        }
    };
    let n = jobs.len();
    let mut failed = 0;
    for (label, job) in jobs {
        let ok = if tx.is_some() {
            conn.execute_batch("SAVEPOINT xq_job").is_ok() && {
                let r = job(&st.db);
                let end = if r.is_ok() {
                    "RELEASE xq_job"
                } else {
                    "ROLLBACK TO xq_job; RELEASE xq_job"
                };
                let _ = conn.execute_batch(end);
                log_err(label, r)
            }
        } else {
            log_err(label, job(&st.db))
        };
        failed += usize::from(!ok);
    }
    if let Some(tx) = tx
        && let Err(e) = tx.commit()
    {
        eprintln!("批量写库提交失败，丢失 {} 条：{e}", n - failed);
    }
}

fn log_err(label: &str, r: Result<(), StoreError>) -> bool {
    match r {
        Ok(()) => true,
        Err(e) => {
            eprintln!("写库失败（{label}）：{e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xq-writer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("t.db")
    }

    fn rows(db: &Db) -> i64 {
        db.conn()
            .query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
            .unwrap()
    }

    fn put(k: usize) -> impl FnOnce(&Db) -> Result<(), StoreError> + Send + 'static {
        move |db| db.settings_set(&format!("t.{k}"), "1")
    }

    #[test]
    fn queued_writes_wait_for_the_batch_and_reads_see_them_after() {
        let path = tmp("delay");
        let w = DbWriter::with_delay(Db::open(&path).unwrap(), Duration::from_millis(200));
        let reader = Db::open_reader(&path).unwrap();
        w.enqueue("t", put(1));
        w.enqueue("t", put(2));
        assert_eq!(rows(&reader), 0, "还没到时间，读不到");
        let t0 = Instant::now();
        while rows(&reader) < 2 {
            assert!(t0.elapsed() < Duration::from_secs(5), "到时间了还没提交");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(w.pending(), 0);
    }

    #[test]
    fn full_batch_commits_at_once() {
        let path = tmp("full");
        let w = DbWriter::with_delay(Db::open(&path).unwrap(), Duration::from_secs(3600));
        let reader = Db::open_reader(&path).unwrap();
        for k in 0..BATCH_MAX - 1 {
            w.enqueue("t", put(k));
        }
        assert_eq!(rows(&reader), 0);
        w.enqueue("t", put(BATCH_MAX));
        assert_eq!(rows(&reader), BATCH_MAX as i64);
    }

    #[test]
    fn sync_writes_commit_queued_ones_first() {
        let path = tmp("sync");
        let w = DbWriter::with_delay(Db::open(&path).unwrap(), Duration::from_secs(3600));
        let reader = Db::open_reader(&path).unwrap();
        w.enqueue("t", put(1));
        // 同步写入能看到排在它前面的数据
        let seen = w.write_sync(|db| {
            db.settings_set("t.sync", "1").unwrap();
            db.settings_get("t.1").unwrap()
        });
        assert_eq!(seen.as_deref(), Some("1"));
        assert_eq!(rows(&reader), 2, "返回时已经落盘");
    }

    #[test]
    fn a_failing_job_only_loses_itself() {
        let path = tmp("fail");
        let w = DbWriter::with_delay(Db::open(&path).unwrap(), Duration::from_secs(3600));
        w.enqueue("t", put(1));
        w.enqueue("bad", |db| {
            db.settings_set("t.half", "1")?;
            db.conn()
                .execute("INSERT INTO no_such_table VALUES (1)", [])?;
            Ok(())
        });
        w.enqueue("t", put(2));
        w.flush();
        let reader = Db::open_reader(&path).unwrap();
        assert_eq!(rows(&reader), 2, "失败任务写了一半的也回滚");
        assert_eq!(reader.settings_get("t.half").unwrap(), None);
    }

    #[test]
    fn dropping_the_writer_commits_the_rest() {
        let path = tmp("drop");
        let w = DbWriter::with_delay(Db::open(&path).unwrap(), Duration::from_secs(3600));
        w.enqueue("t", put(1));
        drop(w);
        assert_eq!(rows(&Db::open_reader(&path).unwrap()), 1);
    }

    #[test]
    fn reader_refuses_writes_in_debug_builds() {
        let path = tmp("ro");
        drop(Db::open(&path).unwrap());
        let reader = Db::open_reader(&path).unwrap();
        assert_eq!(
            reader.settings_set("t.x", "1").is_err(),
            cfg!(debug_assertions)
        );
    }
}
