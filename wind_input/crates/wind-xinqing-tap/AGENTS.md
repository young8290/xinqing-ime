# wind-xinqing-tap

心晴在核心服务里的 XQP 服务端：采集钩子、隐私闸门、命名管道（产品书 17 第 1 节、10 第 2 节、04 FR-SEN-01～07）。
与 17 写法不同的地方见 [ADR 0009](../../../docs/adr/0009-核心侧XQP服务端的实现约定.md)。

## 模块

| 文件 | 内容 |
|---|---|
| `lib.rs` | 对外 API：`Tap::start`、`hook_*`、无痕、改写、启停 |
| `guard.rs` | Hub 守护（A-06，17 第 1.5 节）：纯状态机 `Machine`（时间由调用方给，单元测试覆盖 TC-OPS-03）+ `xq-hub-guard` 线程外壳 `HubGuard`（`on_link_change` 在连接接上 / 断开时回调，协调器据此刷新工具栏）、`ExeLauncher` |
| `gate.rs` | 隐私闸门（全原子量）与应用黑白名单，单元测试逐行覆盖 17 第 1.3 节真值表 |
| `recent.rs` | 改写用的“最近上屏”缓冲（10 第 3.1 节），只在内存里 |
| `server.rs` | 接受、握手、读线程、写线程、心跳 |
| `transport.rs` | 传输抽象与本机 TCP 实现（测试、非 Windows 联调） |
| `pipe.rs` | Windows 命名管道：当前用户 SID 的 ACL、`PIPE_REJECT_REMOTE_CLIENTS`、重叠 I/O |

线程：`xq-tap-accept`、`xq-tap-writer`（唯一的写者），每个连接一个 `xq-tap-reader`；协调器另起 `xq-hub-guard`。没有 tokio（C-PLT-03）。

## 接入协调器

A-04 已接好，代码在 `wind-coordinator/src/xinqing.rs`（钩子点见那边 AGENTS.md）：服务启动时
`Tap::start`，管道名按 `pipe_suffix` 选 `PIPE_NAME` 或 `PIPE_NAME_DEV`，设了 `XQ_XQP_TCP` 改走本机
TCP；重启前调 `shutdown`。按键与组字在 `handle_key_event_policed` 前后各取一次（组字长度、页码）
快照，差异推出 `comp{update/cancel/clear}`、`cand{page}`；选词在上屏钩子里发 `cand{select}`。

A-05 接上了输入法配置 `[xinqing]`（`enabled`、`app_blocklist`、`app_allowlist`、`pause_hotkey`、
`remember_pause`，出厂黑名单 `XINQING_DEFAULT_BLOCKLIST` 与产品书 04 FR-SEN-05 一致）和无痕的菜单、快捷键；
Hub 下发的 `pause` 也会记进 state.toml。握手完成时核心若处于无痕，补发 `pause_changed{on:true}`（ADR 0009 第 7 条）。

A-06 接上了 Hub 守护（`xinqing.hub_autostart`，与总开关一起决定要不要 Hub）和下行处理：`mood`、`badge`、
`pending` 记进协调器的 `xinqing::hub_view()`；Hub 发的 `bye` 在断开后交给回调，守护据此不重拉。

A-07 接上了主菜单“心晴”分组、`tip` 气泡和工具栏天气按钮（无痕时淡显）。A-08 接上了改写模式：
协调器进入时调 `take_recent()`（同意 ⑥ 但还没同意时用 `rewrite_consented()` 区分提示）、`send_rewrite_req`、
`send_rewrite_done`，下行 `rewrite_result` / `rewrite_fail` 交给协调器；选区变化调 `hook_selection_changed()`。

还没接的：

- 安全桌面（`set_secure_desktop` 没有调用方）。
- 小精灵闭眼（Hub 侧）。
- 组字长度只看 `input_buffer`，临时拼音、临时英文等独占模式的缓冲不计。

## 约束

- 钩子里不做 IO、不等锁、不打 info 日志；info 日志不得包含按键、上屏文字或进程名。
- Hub 未连接时事件直接丢弃，不缓存（FR-SEN-07）；同意状态只以 Hub 下发的 `cfg` 为准。
- 管道 ACL 不能照抄 wind-rpc 的 SDDL（那个放行所有人）。

## 测试

`cargo test -p wind-xinqing-tap`：单元测试覆盖闸门与缓冲；`tests/tap.rs` 用本机 TCP 扮演 Hub，覆盖握手、版本不符、闸门、`send_text`、无痕（含连上前已开启）、下行回调、Hub 的 `bye`、新连接顶替、断线撤销同意、队列满丢弃与心跳计数、停用、改写。
命名管道部分只能在 Windows 上验证；Linux 上可用 `cargo clippy -p wind-xinqing-tap --target x86_64-pc-windows-gnu` 做编译检查。
