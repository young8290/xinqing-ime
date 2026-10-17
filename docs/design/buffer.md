# Buffer（上屏前的文本工作台）

> 状态：P1 纯逻辑已实现（`moodflow_input/crates/moodflow-buffer`）；Android 门面（`moodflow-mobile::buffer`）与
> 键盘内 Buffer 栏已搭好；Windows 已接入协调器（`handle_buffer`）与浮窗（`moodflow-ui::buffer_panel`）。
> 两端都**未真机验证**。
> 来源：[XSIME 吸收计划](xsime-feature-port-plan.md) §2.1；行为契约取自 XSIME 在 macOS / Windows / Android 上验证过的规则。

## 1. 是什么

开启 Buffer 后，确认的候选**先进 Buffer 而不是宿主**。用户在 Buffer 里攒好、改好，再按块投递到绑定的输入框。
它是二批 AI 生成、润色、翻译的地基：这些都作用于 Buffer 里的原文，结果经用户确认后再投递。
在插件体系里，它们落为 `BufferAction` 插件（[插件系统设计](plugin-system.md) §3）。

## 2. 契约

| # | 规则 | crate 里的落点 |
|---|---|---|
| 1 | 每次确认的候选是一个块；连续直输的英文合成一块 | `push_phrase` / `push_latin` |
| 2 | 退格：英文逐字删，候选整块删 | `backspace` |
| 3 | 投递授权绑定目标（输入框 ID + 修订号），**显式绑定**，不自动跟随焦点 | `bind` / `focus_changed` |
| 4 | 换框、失焦、隐藏面板、切输入法撤销授权；在途投递作废，旧内容绝不落到新目标 | `focus_changed` / `suspend` / `is_authorized` |
| 5 | 宿主拒收：保留可重试，不算已消费 | `finish(Rejected)` |
| 6 | 结果不明：保留并**暂停投递**，不自动重发（防重复上屏），用户确认后继续 | `finish(Unknown)` / `release_hold` |
| 7 | 发送键：有组字时只确认组字，同一次按键不顺带发送 | `send_key_action` |
| 8 | 「全部投递」按下时冻结队列，之后追加的块不在本次内 | `begin_all` |
| 9 | 块之间的视觉间隔不变成空格 | `Delivery::text` 直接拼接 |
| 10 | 密码框与私密字段禁用 | `bind(.., Private)` 拒绝并撤销绑定 |
| 11 | 容量上限 16,384 个 UTF-16 码元，超限整段拒收不截断 | `check_capacity` |
| 12 | 正文是临时数据，升级、重启、重置直接丢弃 | `discard` |
| 13 | 投递中的块不能被编辑；新输入另起一块 | `check_last_editable` / `push_latin` |

投递是两段式：`begin_*` 取出文本并锁住块 → 宿主写入前 `is_authorized` 复核 → 写完 `finish` 报告结果。

## 3. 两端如何接入（P2 / P3）

| | Windows（TSF） | Android（InputMethodService） |
|---|---|---|
| `Target.id` | TSF 文档/上下文身份 | `EditorInfo` 的字段身份 |
| `Target.revision` | 上下文重建时递增 | 每次 `onStartInput` 递增 |
| 绑定 | 在宿主输入框按绑定快捷键（XSIME 默认 Alt+Shift+B） | 打开键盘内 Buffer 栏时绑定当前字段 |
| 失焦时 | `suspend`：保留内容，需重新绑定 | `discard`：换字段/隐藏键盘即清空（与 XSIME Android 一致） |
| 写入 | edit session 内先 `is_authorized` 再写 | `commitText` 前 `is_authorized`，返回值映射为 Accepted / Rejected |
| 结果不明 | 写入异常且无法确认 → `Unknown` | 连接已断开 → `Unknown` |
| 私密字段 | 输入范围为密码 | `TYPE_TEXT_VARIATION_PASSWORD` 等 |

发送键、绑定键的出厂值放标量配置字段（如 `keys.toggle_buffer`、`keys.buffer_send`），不放 `keys.key_actions`，
保证用户删得掉（AGENTS.md「出厂预置的绑定/开关」）。

## 4. Windows 接入现状（2026-10-17）

- 绑定 `keys.toggle_buffer`（出厂 Alt+Shift+B）：打开并绑定当前输入框；暂停后再按重新绑定；已绑定时再按关闭（内容保留）。
- 发送 `keys.buffer_send`（出厂 Ctrl+Shift+Enter）：经按键响应同步写入，按 Accepted 结算；有组字时只提示，不顺带发送。
  Buffer 关着时这个键落回常规链路。
- 截获点：按键动作出口（`handle_key_event_policed`），在插件上屏变换与换行改写之前；收进 Buffer 后宿主只收到一次空上屏。
- 浮窗贴在光标行上方，右键菜单：全部发送、删除最后一块、清空、关闭。
- **已知差距**：
  1. 目标身份是 TSF 客户端令牌（进程 + 实例），同一进程内换框识别不到；补偿做法是任何有效失焦都暂停，回框后按一次绑定键。
     要做到真正按输入框区分，需要 TSF 在 `FocusGainedPayload` 里带上文档/上下文身份。
  2. 推送通道（菜单发送）没有回执，「结果不明 → 暂停不重发」暂时区分不了，按 Accepted 处理。
  3. 英文直输不经协调器出字，不会合进 Buffer（契约 1 的后半句在 Windows 上尚未实现）。
  4. 长按逐块投递、节奏投递未做。

## 5. 下一步

1. TSF：焦点事件带文档身份；推送上屏加回执。
2. Android：协调器接入后改走同一套截获（目前 Android 上 Buffer 直接截获宿主侧的上屏指令）。
3. 长按发送逐块投递（默认约 1.2 秒，可配 500–3000 ms）、可选 1/2/3/5 秒节奏投递：在协调器里用 `begin_next` 循环实现，本 crate 不管计时。
