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

## 接入协调器

A-04 已接好，代码在 `wind-coordinator/src/xinqing.rs`（钩子点见那边 AGENTS.md）：服务启动时
`Tap::start`，管道名按 `pipe_suffix` 选 `PIPE_NAME` 或 `PIPE_NAME_DEV`，设了 `XQ_XQP_TCP` 改走本机
TCP；重启前调 `shutdown`。按键与组字在 `handle_key_event_policed` 前后各取一次（组字长度、页码）
快照，差异推出 `comp{update/cancel/clear}`、`cand{page}`；选词在上屏钩子里发 `cand{select}`。

A-05 接上了输入法配置 `[xinqing]`（`enabled`、`app_blocklist`、`app_allowlist`、`pause_hotkey`、
`remember_pause`，出厂黑名单 `XINQING_DEFAULT_BLOCKLIST` 与产品书 04 FR-SEN-05 一致）和无痕的菜单、快捷键；
Hub 下发的 `pause` 也会记进 state.toml。握手完成时核心若处于无痕，补发 `pause_changed{on:true}`（ADR 0009 第 7 条）。

还没接的：

- `downlink` 除 `pause` 外只记 debug 日志；`mood`、`badge`、`tip` 的界面处理在 A-06/A-07。
- 小精灵闭眼、工具栏变灰（FR-SEN-06 第 3 条）、入口按钮 `send_open`（A-07）、安全桌面、改写模式（A-08）。
- 组字长度只看 `input_buffer`，临时拼音、临时英文等独占模式的缓冲不计。

## 约束

- 钩子里不做 IO、不等锁、不打 info 日志；info 日志不得包含按键、上屏文字或进程名。
- Hub 未连接时事件直接丢弃，不缓存（FR-SEN-07）；同意状态只以 Hub 下发的 `cfg` 为准。
- 管道 ACL 不能照抄 wind-rpc 的 SDDL（那个放行所有人）。

## 测试

`cargo test -p wind-xinqing-tap`：单元测试覆盖闸门与缓冲；`tests/tap.rs` 用本机 TCP 扮演 Hub，覆盖握手、版本不符、闸门、`send_text`、无痕（含连上前已开启）、下行回调、新连接顶替、断线撤销同意、队列满丢弃与心跳计数、停用、改写。
命名管道部分只能在 Windows 上验证；Linux 上可用 `cargo clippy -p wind-xinqing-tap --target x86_64-pc-windows-gnu` 做编译检查。
