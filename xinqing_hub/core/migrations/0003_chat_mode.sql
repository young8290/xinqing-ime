-- 心晴 Hub 数据库 v3：会话的对话方式（FR-CHT-06 快捷指令，docs/adr/0021）。
-- “我只是想吐槽”切到 vent、“帮我理一理”切到 organize，整段会话有效；已有会话都是平常的对话。
ALTER TABLE chat_session ADD COLUMN mode TEXT NOT NULL DEFAULT 'normal'
  CHECK (mode IN ('normal','vent','organize'));
