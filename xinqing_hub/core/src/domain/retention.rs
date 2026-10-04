//! 保留与自动清理（FR-DAT-02）：按 09 第 1 节的保留期限删除过期数据，再做增量 `VACUUM`。
//!
//! 每条规则单独执行、单独提交，一条失败不影响其他规则（“清理任务失败只记日志，下次继续”）。
//! 什么时候跑由外壳决定：每天 04:30，另外 Hub 启动后补跑一次（docs/adr/0011 第 1 条）。
//!
//! 不在这里处理的表：`baseline`（滚动 7 天由基线重算维护，B-04）、`diary` / `memory` /
//! `collection`（直到用户删除）、`consent`（直到删除全部数据）、已添加的日程和未完成的待办
//! （直到用户删除）。`net_log` 平时在写入时就只留 200 条，这里再兜底一次。

use chrono::{DateTime, Days, Duration, Local, NaiveTime};
use rusqlite::{Connection, params};

use crate::infra::store::{Db, NET_LOG_KEEP, StoreError};

const DAY_MS: i64 = 86_400_000;

/// 每天几点清理（FR-DAT-02）。
pub const DAILY_AT: (u32, u32) = (4, 30);

/// 可由用户调整的保留期限；其余期限写死在 [`RULES`] 里，与 09 第 1 节一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// 对话（D-11）：默认 90 天，可选 30 / 90 / 365 天，`None` 为永久（FR-CHT-08，设置键 `chat.retention_days`）
    pub chat_days: Option<u32>,
    /// 每日汇总（D-08）：1 年，可调
    pub daily_summary_days: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            chat_days: Some(90),
            daily_summary_days: 365,
        }
    }
}

/// 一条清理规则：数据清单编号、说明、SQL。SQL 只有一个参数 `?1`，即截止时刻
/// （Unix 毫秒；`daily_summary` 是 `YYYY-MM-DD`），早于它的数据按规则处理。
struct Rule {
    data: &'static str,
    what: &'static str,
    cutoff: Cutoff,
    sql: &'static str,
}

#[derive(Clone, Copy)]
enum Cutoff {
    Days(i64),
    ChatDays,
    SummaryDays,
    /// 不按时间：`net_log` 按条数
    Rows,
}

const RULES: &[Rule] = &[
    Rule {
        data: "D-06",
        what: "window_features：30 天",
        cutoff: Cutoff::Days(30),
        sql: "DELETE FROM window_features WHERE end_ts < ?1",
    },
    Rule {
        data: "D-07",
        what: "mood_state：30 天",
        cutoff: Cutoff::Days(30),
        sql: "DELETE FROM mood_state WHERE ts < ?1",
    },
    Rule {
        data: "D-08",
        what: "daily_summary：1 年（可调）",
        cutoff: Cutoff::SummaryDays,
        sql: "DELETE FROM daily_summary WHERE date < ?1",
    },
    Rule {
        data: "D-10",
        what: "comfort_log：90 天",
        cutoff: Cutoff::Days(90),
        sql: "DELETE FROM comfort_log WHERE ts < ?1",
    },
    // 整个会话按最后一条消息的时间过期，不拆开删半个会话（ADR 0011 第 3 条）；
    // chat_message 随 ON DELETE CASCADE 一起删除。
    Rule {
        data: "D-11",
        what: "chat_session / chat_message：默认 90 天",
        cutoff: Cutoff::ChatDays,
        sql: "DELETE FROM chat_session WHERE coalesce(
                (SELECT max(ts) FROM chat_message m WHERE m.session_id = chat_session.id),
                created_ts) < ?1",
    },
    Rule {
        data: "D-12",
        what: "schedule：待确认 7 天",
        cutoff: Cutoff::Days(7),
        sql: "DELETE FROM schedule WHERE status = 'pending' AND created_ts < ?1",
    },
    // “已忽略：只存哈希”：写入时就应只留哈希，这里兜底清掉内容字段（ADR 0011 第 4 条）。
    // 与时间无关，?1 恒成立。
    Rule {
        data: "D-12",
        what: "schedule：已忽略只存哈希",
        cutoff: Cutoff::Days(0),
        sql: "UPDATE schedule SET title = NULL, date = NULL, time = NULL, end_time = NULL, location = NULL
              WHERE status = 'ignored' AND ?1 IS NOT NULL
                AND coalesce(title, date, time, end_time, location) IS NOT NULL",
    },
    Rule {
        data: "D-16",
        what: "reminder_log：90 天",
        cutoff: Cutoff::Days(90),
        sql: "DELETE FROM reminder_log WHERE ts < ?1",
    },
    Rule {
        data: "D-17",
        what: "feedback：90 天",
        cutoff: Cutoff::Days(90),
        sql: "DELETE FROM feedback WHERE ts < ?1",
    },
    Rule {
        data: "D-18",
        what: "safety_log：90 天",
        cutoff: Cutoff::Days(90),
        sql: "DELETE FROM safety_log WHERE ts < ?1",
    },
    Rule {
        data: "D-20",
        what: "net_log：最近 200 条",
        cutoff: Cutoff::Rows,
        sql: "DELETE FROM net_log WHERE id NOT IN (SELECT id FROM net_log ORDER BY id DESC LIMIT ?1)",
    },
    Rule {
        data: "D-24",
        what: "self_report：1 年",
        cutoff: Cutoff::Days(365),
        sql: "DELETE FROM self_report WHERE ts < ?1",
    },
    Rule {
        data: "D-25",
        what: "todo：待确认 7 天",
        cutoff: Cutoff::Days(7),
        sql: "DELETE FROM todo WHERE status = 'pending' AND created_ts < ?1",
    },
    // 已完成：完成 7 天后归档、30 天后删除，都从 done_ts 算（ADR 0011 第 5 条）。
    // 先删后归档，同一次清理里不会把刚归档的又算一遍。
    Rule {
        data: "D-25",
        what: "todo：完成 30 天后删除",
        cutoff: Cutoff::Days(30),
        sql: "DELETE FROM todo WHERE status IN ('done','archived') AND done_ts < ?1",
    },
    Rule {
        data: "D-25",
        what: "todo：完成 7 天后归档",
        cutoff: Cutoff::Days(7),
        sql: "UPDATE todo SET status = 'archived' WHERE status = 'done' AND done_ts < ?1",
    },
    Rule {
        data: "D-26",
        what: "letter：1 年",
        cutoff: Cutoff::Days(365),
        sql: "DELETE FROM letter WHERE created_ts < ?1",
    },
    Rule {
        data: "D-28",
        what: "rewrite_log：90 天",
        cutoff: Cutoff::Days(90),
        sql: "DELETE FROM rewrite_log WHERE ts < ?1",
    },
    Rule {
        data: "D-29",
        what: "focus_session：1 年",
        cutoff: Cutoff::Days(365),
        sql: "DELETE FROM focus_session WHERE start_ts < ?1",
    },
];

/// 一次清理的结果。只有表名、条数和错误信息，不含任何用户内容（NFR-LOG-01），可以直接记日志。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// `(数据编号, 规则说明, 影响的行数)`，只列影响行数不为 0 的规则
    pub changed: Vec<(&'static str, &'static str, usize)>,
    /// `(规则说明, 错误)`
    pub failed: Vec<(&'static str, String)>,
    /// 增量 `VACUUM` 是否成功
    pub vacuumed: bool,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.failed.is_empty() && self.vacuumed
    }

    /// 一行摘要，给日志用。
    pub fn summary(&self) -> String {
        let n: usize = self.changed.iter().map(|(_, _, n)| n).sum();
        let mut s = format!("数据清理：处理 {n} 行");
        for (data, what, n) in &self.changed {
            s.push_str(&format!("；{data} {what} {n}"));
        }
        for (what, e) in &self.failed {
            s.push_str(&format!("；失败 {what}：{e}"));
        }
        if !self.vacuumed {
            s.push_str("；VACUUM 失败");
        }
        s
    }
}

/// 按 `policy` 清理一次。`now` 来自 `Clock`，演示模式与测试可以指定。
pub fn run(db: &Db, policy: &Policy, now: DateTime<Local>) -> Report {
    let conn = db.conn();
    let now_ms = now.timestamp_millis();
    let mut report = Report::default();
    for rule in RULES {
        let result = match rule.cutoff {
            Cutoff::Days(d) => exec(conn, rule.sql, now_ms - d * DAY_MS),
            Cutoff::ChatDays => match policy.chat_days {
                Some(d) => exec(conn, rule.sql, now_ms - i64::from(d) * DAY_MS),
                None => Ok(0),
            },
            Cutoff::SummaryDays => {
                let cutoff = now
                    .date_naive()
                    .checked_sub_days(Days::new(u64::from(policy.daily_summary_days)))
                    .map(|d| d.format("%Y-%m-%d").to_string());
                match cutoff {
                    Some(c) => exec(conn, rule.sql, c),
                    None => Ok(0),
                }
            }
            Cutoff::Rows => exec(conn, rule.sql, NET_LOG_KEEP),
        };
        match result {
            Ok(0) => {}
            Ok(n) => report.changed.push((rule.data, rule.what, n)),
            Err(e) => report.failed.push((rule.what, e.to_string())),
        }
    }
    // 只有建库时开了 auto_vacuum = INCREMENTAL（`Db::open` 对新库会开）才真正回收空间，否则是空操作。
    report.vacuumed = conn.execute_batch("PRAGMA incremental_vacuum").is_ok();
    report
}

/// 每条规则自成一个语句，SQLite 自动提交，失败不影响已完成的规则。
fn exec(conn: &Connection, sql: &str, p: impl rusqlite::ToSql) -> Result<usize, StoreError> {
    Ok(conn.execute(sql, params![p])?)
}

/// `now` 之后（不含）的下一个清理时刻：当天或次日的 04:30。
pub fn next_run(now: DateTime<Local>) -> DateTime<Local> {
    let at = NaiveTime::from_hms_opt(DAILY_AT.0, DAILY_AT.1, 0).unwrap_or(NaiveTime::MIN);
    let mut date = now.date_naive();
    for _ in 0..3 {
        // 夏令时切换当天这个时刻可能不存在，取最早的合法时刻；不存在就看下一天
        if let Some(t) = date.and_time(at).and_local_timezone(Local).earliest()
            && t > now
        {
            return t;
        }
        match date.succ_opt() {
            Some(d) => date = d,
            None => break,
        }
    }
    now + Duration::days(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .earliest()
            .unwrap()
    }

    fn count(db: &Db, sql: &str) -> i64 {
        db.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn text(db: &Db, sql: &str) -> String {
        db.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// 每张表放一条刚过期、一条刚好没过期的数据（TC-DAT-08）。
    fn seed(db: &Db, now: DateTime<Local>) {
        let ms = |days: i64| now.timestamp_millis() - days * DAY_MS;
        let c = db.conn();
        for (days, id) in [(31, 1), (29, 2)] {
            c.execute(
                "INSERT INTO window_features (id, start_ts, end_ts, app_cat, features_json) VALUES (?1, ?2, ?2, 'doc', '{}')",
                params![id, ms(days)],
            )
            .unwrap();
            c.execute(
                "INSERT INTO mood_state (ts, window_id, state, shown_state, source) VALUES (?1, ?2, 'fluent', 'fluent', 'rule')",
                params![ms(days), id],
            )
            .unwrap();
        }
        for date in ["2025-10-01", "2025-10-06"] {
            c.execute("INSERT INTO daily_summary (date) VALUES (?1)", [date])
                .unwrap();
        }
        for days in [91, 89] {
            c.execute(
                "INSERT INTO comfort_log (ts, state, text, source) VALUES (?1, 'low', '…', 'template')",
                [ms(days)],
            )
            .unwrap();
            for t in [
                "reminder_log (ts, kind, action) VALUES (?1, 'eye', 'done')",
                "feedback (ts, target, verdict) VALUES (?1, 'mood_state', 'unfit')",
                "safety_log (ts, channel) VALUES (?1, 'lexicon')",
                "rewrite_log (ts, source, style, len_in, outcome) VALUES (?1, 'recent', 'gentle', 3, 'replaced')",
            ] {
                c.execute(&format!("INSERT INTO {t}"), [ms(days)]).unwrap();
            }
        }
        for days in [366, 364] {
            c.execute(
                "INSERT INTO self_report (ts, weather, source) VALUES (?1, 'sunny', 'user')",
                [ms(days)],
            )
            .unwrap();
            c.execute(
                "INSERT INTO letter (week_start, content, source, created_ts) VALUES ('2025-10-01', '…', 'template', ?1)",
                [ms(days)],
            )
            .unwrap();
            c.execute(
                "INSERT INTO focus_session (start_ts, planned_min) VALUES (?1, 25)",
                [ms(days)],
            )
            .unwrap();
        }
        // 对话：会话 1 很早创建但最近还在聊，会话 2 最后一条消息 91 天前
        c.execute(
            "INSERT INTO chat_session (id, title, created_ts) VALUES (1, 'a', ?1), (2, 'b', ?1)",
            [ms(200)],
        )
        .unwrap();
        c.execute(
            "INSERT INTO chat_message (session_id, role, content, ts) VALUES (1, 'user', '…', ?1), (1, 'user', '…', ?2), (2, 'user', '…', ?1)",
            params![ms(91), ms(1)],
        )
        .unwrap();
        // 日程：待确认 8 天前 / 6 天前；已添加很早；已忽略还留着标题
        c.execute(
            "INSERT INTO schedule (title, status, source, dedup_hash, created_ts) VALUES
             ('a', 'pending', 'ai', 'h1', ?1), ('b', 'pending', 'ai', 'h2', ?2),
             ('c', 'added', 'ai', 'h3', ?3), ('d', 'ignored', 'ai', 'h4', ?2)",
            params![ms(8), ms(6), ms(400)],
        )
        .unwrap();
        // 待办：待确认 8 天前；完成 31 天前 / 8 天前 / 6 天前；未完成很早
        c.execute(
            "INSERT INTO todo (id, title, status, source, dedup_hash, created_ts, done_ts) VALUES
             (1, 'a', 'pending', 'ai', 'h1', ?1, NULL), (2, 'b', 'done', 'ai', 'h2', ?4, ?2),
             (3, 'c', 'done', 'ai', 'h3', ?4, ?1), (4, 'd', 'done', 'ai', 'h4', ?4, ?3),
             (5, 'e', 'open', 'ai', 'h5', ?4, NULL)",
            params![ms(8), ms(31), ms(6), ms(400)],
        )
        .unwrap();
        for i in 0..(NET_LOG_KEEP + 5) {
            c.execute(
                "INSERT INTO net_log (ts, api, fields, status) VALUES (?1, 'jev', '[]', 'ok')",
                [i],
            )
            .unwrap();
        }
    }

    #[test]
    fn deletes_by_retention_period() {
        let db = Db::open_in_memory().unwrap();
        let now = at(2026, 10, 4, 4, 30);
        seed(&db, now);
        let report = run(&db, &Policy::default(), now);
        assert!(report.is_ok(), "{}", report.summary());

        for (sql, left) in [
            ("SELECT count(*) FROM window_features", 1),
            ("SELECT count(*) FROM mood_state", 1),
            ("SELECT count(*) FROM daily_summary", 1),
            ("SELECT count(*) FROM comfort_log", 1),
            ("SELECT count(*) FROM reminder_log", 1),
            ("SELECT count(*) FROM feedback", 1),
            ("SELECT count(*) FROM safety_log", 1),
            ("SELECT count(*) FROM rewrite_log", 1),
            ("SELECT count(*) FROM self_report", 1),
            ("SELECT count(*) FROM letter", 1),
            ("SELECT count(*) FROM focus_session", 1),
            ("SELECT count(*) FROM net_log", NET_LOG_KEEP),
        ] {
            assert_eq!(count(&db, sql), left, "{sql}");
        }
        assert_eq!(
            count(&db, "SELECT min(date = '2025-10-06') FROM daily_summary"),
            1
        );
        // 会话按最后一条消息过期，消息随会话级联删除
        assert_eq!(text(&db, "SELECT group_concat(id) FROM chat_session"), "1");
        assert_eq!(count(&db, "SELECT count(*) FROM chat_message"), 2);
        // 日程：只删过期的待确认；已忽略的只留哈希
        assert_eq!(
            count(&db, "SELECT count(*) FROM schedule WHERE title IS NOT NULL"),
            2
        );
        assert_eq!(
            count(&db, "SELECT count(*) FROM schedule WHERE dedup_hash = 'h4'"),
            1
        );
        // 待办：1 过期待确认删除、2 完成 31 天删除、3 完成 8 天归档、4 完成 6 天不动、5 未完成不动
        assert_eq!(
            text(
                &db,
                "SELECT group_concat(id || ':' || status, ' ') FROM (SELECT * FROM todo ORDER BY id)"
            ),
            "3:archived 4:done 5:open"
        );

        // 再跑一次什么都不变
        let again = run(&db, &Policy::default(), now);
        assert!(again.changed.is_empty(), "{}", again.summary());
    }

    #[test]
    fn chat_retention_follows_policy() {
        let db = Db::open_in_memory().unwrap();
        let now = at(2026, 10, 4, 4, 30);
        seed(&db, now);
        let forever = Policy {
            chat_days: None,
            ..Policy::default()
        };
        run(&db, &forever, now);
        assert_eq!(count(&db, "SELECT count(*) FROM chat_session"), 2);

        // 30 天：TC-CHT-10 注入 31 天前的会话
        let c = db.conn();
        c.execute(
            "INSERT INTO chat_session (id, title, created_ts) VALUES (3, 'c', ?1)",
            [now.timestamp_millis() - 31 * DAY_MS],
        )
        .unwrap();
        let month = Policy {
            chat_days: Some(30),
            ..Policy::default()
        };
        run(&db, &month, now);
        assert_eq!(text(&db, "SELECT group_concat(id) FROM chat_session"), "1");
    }

    #[test]
    fn one_failing_rule_does_not_stop_the_rest() {
        let db = Db::open_in_memory().unwrap();
        let now = at(2026, 10, 4, 4, 30);
        seed(&db, now);
        db.conn().execute_batch("DROP TABLE letter").unwrap();
        let report = run(&db, &Policy::default(), now);
        assert_eq!(report.failed.len(), 1, "{}", report.summary());
        assert_eq!(report.failed[0].0, "letter：1 年");
        assert_eq!(count(&db, "SELECT count(*) FROM focus_session"), 1);
        assert!(report.summary().contains("失败 letter"));
    }

    #[test]
    fn new_database_reclaims_space_incrementally() {
        let dir = std::env::temp_dir().join(format!("xq-retention-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("xinqing.db");
        let _ = std::fs::remove_file(&path);
        let db = Db::open(&path).unwrap();
        let mode: i64 = db
            .conn()
            .query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, 2, "新库应为 INCREMENTAL");
        assert!(run(&db, &Policy::default(), Local::now()).vacuumed);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_run_is_the_coming_0430() {
        assert_eq!(next_run(at(2026, 10, 4, 3, 0)), at(2026, 10, 4, 4, 30));
        assert_eq!(next_run(at(2026, 10, 4, 4, 30)), at(2026, 10, 5, 4, 30));
        assert_eq!(next_run(at(2026, 10, 4, 23, 59)), at(2026, 10, 5, 4, 30));
    }
}
