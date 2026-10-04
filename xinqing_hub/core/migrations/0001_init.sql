-- 心晴 Hub 数据库 v1（产品书 09 第 3 节）。已发布的迁移脚本禁止修改，新改动另起 0002_*.sql。
-- schema_version 由迁移器维护，不在脚本中写入。


CREATE TABLE window_features (            -- D-06
  id INTEGER PRIMARY KEY,
  start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL,
  app_cat TEXT NOT NULL CHECK (app_cat IN ('chat','doc','code','browser','other')),
  features_json TEXT NOT NULL,            -- 只含数值特征，禁止出现文本
  hints TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_wf_end ON window_features(end_ts);

CREATE TABLE mood_state (                 -- D-07
  id INTEGER PRIMARY KEY,
  ts INTEGER NOT NULL,
  window_id INTEGER REFERENCES window_features(id) ON DELETE SET NULL,
  state TEXT NOT NULL,                    -- fluent/hesitant/low/agitated/tired/unknown
  shown_state TEXT NOT NULL,              -- 融合后实际显示的状态
  probs_json TEXT, valence REAL, need_comfort REAL,
  source TEXT NOT NULL CHECK (source IN ('jev','rule')),
  model TEXT, question_ver TEXT, latency_ms INTEGER
);
CREATE INDEX idx_ms_ts ON mood_state(ts);

CREATE TABLE daily_summary (              -- D-08
  date TEXT PRIMARY KEY,
  typing_min INTEGER NOT NULL DEFAULT 0, keystrokes INTEGER NOT NULL DEFAULT 0,
  state_dist_json TEXT, dominant_state TEXT, avg_valence REAL,
  comforts INTEGER NOT NULL DEFAULT 0, rests_due INTEGER NOT NULL DEFAULT 0,
  rests_done INTEGER NOT NULL DEFAULT 0, water INTEGER NOT NULL DEFAULT 0,
  focus_min INTEGER NOT NULL DEFAULT 0,
  last_active_ts INTEGER,                 -- D-31 当晚停止打字时间（18:00–次日 06:00 的最后活跃分钟）
  sunny_points INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE baseline (                   -- D-09
  feature TEXT NOT NULL, bucket TEXT NOT NULL CHECK (bucket IN ('day','night')),
  med REAL NOT NULL, mad REAL NOT NULL, n INTEGER NOT NULL, updated_ts INTEGER NOT NULL,
  PRIMARY KEY (feature, bucket)
);

CREATE TABLE comfort_log (                -- D-10
  id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, state TEXT NOT NULL,
  text TEXT NOT NULL, source TEXT NOT NULL CHECK (source IN ('llm','template')),
  template_id TEXT, model TEXT, prompt_ver TEXT,
  feedback TEXT CHECK (feedback IN ('useful','unfit','mute') OR feedback IS NULL)
);

CREATE TABLE chat_session (id INTEGER PRIMARY KEY, title TEXT NOT NULL,
  created_ts INTEGER NOT NULL, safe_mode INTEGER NOT NULL DEFAULT 0);
CREATE TABLE chat_message (               -- D-11
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES chat_session(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user','assistant','system_notice')),
  content TEXT NOT NULL, ts INTEGER NOT NULL,
  ai_generated INTEGER NOT NULL DEFAULT 0, model TEXT, prompt_ver TEXT
);

CREATE TABLE schedule (                   -- D-12
  id INTEGER PRIMARY KEY,
  title TEXT, date TEXT, time TEXT, end_time TEXT,
  all_day INTEGER NOT NULL DEFAULT 0, location TEXT, is_deadline INTEGER NOT NULL DEFAULT 0,
  remind_offsets TEXT NOT NULL DEFAULT '[600]',   -- 秒，JSON 数组
  status TEXT NOT NULL CHECK (status IN ('pending','added','ignored','expired')),
  source TEXT NOT NULL CHECK (source IN ('ai','manual')),
  flags TEXT NOT NULL DEFAULT '',          -- adjusted / maybe_past / confirm_date
  dedup_hash TEXT NOT NULL, created_ts INTEGER NOT NULL
);
CREATE INDEX idx_sch_hash ON schedule(dedup_hash);

CREATE TABLE diary (                      -- D-13
  id INTEGER PRIMARY KEY, date TEXT NOT NULL, content TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('ai_draft','ai_edited','manual')),
  created_ts INTEGER NOT NULL, updated_ts INTEGER NOT NULL
);

CREATE TABLE memory (id INTEGER PRIMARY KEY, content TEXT NOT NULL CHECK (length(content) <= 100),
  created_ts INTEGER NOT NULL);                                        -- D-14

CREATE TABLE consent (                    -- D-15
  id INTEGER PRIMARY KEY, item TEXT NOT NULL, policy_ver INTEGER NOT NULL,
  granted INTEGER NOT NULL, ts INTEGER NOT NULL
);

CREATE TABLE reminder_log (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
  kind TEXT NOT NULL, action TEXT NOT NULL);                           -- D-16
CREATE TABLE feedback (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
  target TEXT NOT NULL, target_id INTEGER, verdict TEXT NOT NULL);      -- D-17
CREATE TABLE safety_log (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
  channel TEXT NOT NULL CHECK (channel IN ('lexicon','jev','both')));   -- D-18（不存内容）
CREATE TABLE net_log (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, api TEXT NOT NULL,
  model TEXT, fields TEXT NOT NULL, latency_ms INTEGER, status TEXT NOT NULL,
  tokens_in INTEGER, tokens_out INTEGER);                               -- D-20（不存内容）
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);      -- 设置

CREATE TABLE self_report (                -- D-24
  id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
  weather TEXT NOT NULL CHECK (weather IN ('sunny','cloudy','rain','storm','night','unsure')),
  note TEXT CHECK (note IS NULL OR length(note) <= 50),   -- 只存本地，永不出网
  auto_state TEXT,                        -- 同一时刻自动判断的状态，用于比较
  source TEXT NOT NULL CHECK (source IN ('user','esm'))
);

CREATE TABLE todo (                       -- D-25
  id INTEGER PRIMARY KEY, title TEXT, due_date TEXT,
  status TEXT NOT NULL CHECK (status IN ('pending','open','done','archived','ignored')),
  source TEXT NOT NULL CHECK (source IN ('ai','manual')),
  dedup_hash TEXT NOT NULL, created_ts INTEGER NOT NULL, done_ts INTEGER
);

CREATE TABLE letter (                     -- D-26
  id INTEGER PRIMARY KEY, week_start TEXT NOT NULL, content TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('llm','template')),
  model TEXT, prompt_ver TEXT, read INTEGER NOT NULL DEFAULT 0, created_ts INTEGER NOT NULL
);

CREATE TABLE rewrite_log (                -- D-28（不存文字）
  id INTEGER PRIMARY KEY, ts INTEGER NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('recent','clipboard','selection')),
  style TEXT NOT NULL, chosen INTEGER,      -- NULL 表示取消
  len_in INTEGER NOT NULL, len_out INTEGER, outcome TEXT NOT NULL   -- replaced/inserted/copied/cancelled/failed
);

CREATE TABLE focus_session (              -- D-29
  id INTEGER PRIMARY KEY, start_ts INTEGER NOT NULL, planned_min INTEGER NOT NULL,
  actual_min INTEGER, keys INTEGER, chars INTEGER
);

CREATE TABLE collection (                 -- D-30
  item TEXT PRIMARY KEY, unlocked_ts INTEGER, equipped INTEGER NOT NULL DEFAULT 0
);
