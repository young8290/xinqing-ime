# xq-sim
- 职责：扮演核心服务，按 XQP 握手（10 第 2.3 节）把 JSONL 脚本回放给 Hub（FR-DMO-01）；`--listen` 以 Hub 身份录制并去掉 `text`（FR-DMO-02）。
- 用法：`xq-sim --script <脚本> [--baseline <基线.toml>] [--start-at HH:MM] [--speed 5] [--pipe xinqing_tap_dev | --tcp 127.0.0.1:18765 | --stdout] [--check]`。
- 脚本：`scripts/*.jsonl` 由 `gen_synthetic.py` 生成，**只用于开发和回归**；E-STATE 评测脚本必须真实录制（eval/datasets/e_state_scripts.md）。
- `--baseline` 校验固定基线 TOML 并把版本写入调试日志；XQP v1 的 `cfg` 没有基线字段，因此不会把基线内容发送给 Hub。
- `--start-at HH:MM` 将脚本时间轴平移到指定本地时刻，便于复现昼夜规则；时间早于当前时刻时按当天起点计算。
- 暂停 / 单步、Windows 管道客户端版 `--listen` 在接入核心后补充。
