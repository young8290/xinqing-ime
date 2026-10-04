# xq-sim
- 职责：扮演核心服务，按 XQP 握手（10 第 2.3 节）把 JSONL 脚本回放给 Hub（FR-DMO-01）；`--listen` 以 Hub 身份录制并去掉 `text`（FR-DMO-02）。
- 用法：`xq-sim --script <脚本> [--speed 5] [--pipe xinqing_tap_dev | --tcp 127.0.0.1:18765 | --stdout] [--check]`。
- 脚本：`scripts/*.jsonl` 由 `gen_synthetic.py` 生成，**只用于开发和回归**；E-STATE 评测脚本必须真实录制（eval/datasets/e_state_scripts.md）。
- 待补：`--baseline`、`--start-at` 目前由 `xq-replay` 提供；暂停 / 单步、Windows 管道客户端版 `--listen` 在接入核心后补充。
