-- 心晴 Hub 数据库 v2：暖心话记录区分触发方式（C-04，docs/adr/0015 第 1 条）。
-- 主动关怀（FR-CMF-01）计入每日上限和冷却；负面自评后的回应（FR-STA-10 第 2 条）不计入。
-- 已有记录都来自主动关怀，默认值取 'auto'。
ALTER TABLE comfort_log ADD COLUMN trigger TEXT NOT NULL DEFAULT 'auto'
  CHECK (trigger IN ('auto','self_report'));
CREATE INDEX idx_cl_ts ON comfort_log(ts);
