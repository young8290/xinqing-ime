# xqp
- 职责：XQP 协议 v1 的消息类型与帧编解码（10 第 2 节，FR-SEN-07）。
- 对外接口：`Up` / `Down` 枚举、`encode` / `read_frame` / `decode_up` / `decode_down`、`Down::validate`。
- 依赖：serde、serde_json、thiserror。
- 契约：字段与 `protocol/xqp.schema.json` 一一对应；CI 用 xq-sim `--stdout` 的输出对照 schema 校验。
- 测试：`cargo test -p xqp`（10 第 2.6 节示例会话、帧上限、去文本）。
- 隐私检查：`Up::strip_text` 用于录制时去掉 `text`（FR-DMO-02）。
