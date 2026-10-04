# wind-xinqing-tap

心晴在核心服务里的 XQP 服务端：采集钩子、隐私闸门、命名管道（产品书 17 第 1 节、10 第 2 节、04 FR-SEN-01～07）。
与 17 写法不同的地方见 [ADR 0009](../../../docs/adr/0009-核心侧XQP服务端的实现约定.md)。

## 模块

| 文件 | 内容 |
|---|---|
| `lib.rs` | 对外 API：`Tap::start`、`hook_*`、无痕、改写、启停 |
| `gate.rs` | 隐私闸门（全原子量）与应用黑白名单，单元测试逐行覆盖 17 第 1.3 节真值表 |
| `recent.rs` | 改写用的“最近上屏”缓冲（10 第 3.1 节），只在内存里 |
| `server.rs` | 接受、握手、读线程、写线程、心跳 |
| `transport.rs` | 传输抽象与本机 TCP 实现（测试、非 Windows 联调） |
| `pipe.rs` | Windows 命名管道：当前用户 SID 的 ACL、`PIPE_REJECT_REMOTE_CLIENTS`、重叠 I/O |

线程：`xq-tap-accept`、`xq-tap-writer`（唯一的写者），每个连接一个 `xq-tap-reader`。没有 tokio（C-PLT-03）。

## 接入协调器（A-04 要做的事）

- 启动：`Tap::start(TapConfig::new(ime_ver, Endpoint::Pipe(name)), downlink)`，`name` 按 `wind_config::variant::pipe_suffix()` 选 `xqp::PIPE_NAME` 或 `xqp::PIPE_NAME_DEV`；`xinqing.enabled`、名单来自配置。
- 钩子位置见 17 第 1.2 节：`handle_key_event` → `hook_key`，`record_commit_ks` → `hook_commit`，`handle_focus_gained` / `handle_input_state_report` → `hook_focus`，组字与候选 → `hook_comp` / `hook_cand`。钩子不阻塞，可在状态锁内调用。
- `downlink` 在读线程上回调 `pause`、`mood`、`badge`、`tip`、`pending`、`rewrite_result`、`rewrite_fail`，协调器在状态锁内更新并刷新界面。
- 菜单或快捷键切换无痕调 `set_paused(on, by)`；入口按钮调 `send_open`；安全桌面调 `set_secure_desktop`；核心退出前调 `shutdown`。
- 改写模式：进入时 `take_recent()`，`rewrite_enabled()` 为假时只弹同意询问；发请求 `send_rewrite_req`，结束 `send_rewrite_done`。

## 约束

- 钩子里不做 IO、不等锁、不打 info 日志；info 日志不得包含按键、上屏文字或进程名。
- Hub 未连接时事件直接丢弃，不缓存（FR-SEN-07）；同意状态只以 Hub 下发的 `cfg` 为准。
- 管道 ACL 不能照抄 wind-rpc 的 SDDL（那个放行所有人）。

## 测试

`cargo test -p wind-xinqing-tap`：单元测试覆盖闸门与缓冲；`tests/tap.rs` 用本机 TCP 扮演 Hub，覆盖握手、版本不符、闸门、`send_text`、无痕、下行回调、新连接顶替、断线撤销同意、队列满丢弃与心跳计数、停用、改写。
命名管道部分只能在 Windows 上验证；Linux 上可用 `cargo clippy -p wind-xinqing-tap --target x86_64-pc-windows-gnu` 做编译检查。
