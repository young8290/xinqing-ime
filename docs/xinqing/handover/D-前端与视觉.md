# D 前端与视觉 · 交接文档

> 负责范围（产品书 13 第 1 节）：WGT 小组件、DSH 看板、SET 设置、ONB 引导、NTF 系统通知、REV-04 晴天收集的界面、设计规范；
> 另是前后端绑定 `xinqing_hub/src/api/bindings.ts` 的契约负责人（13 第 3.1 节）。
> 任务清单与估算见产品书 16 第 2.4 节。本文件随 D 的每个 PR 更新，任务中途换人时按产品书 13 第 3.1 节“交接”直接看这里。
> 最后更新：2026-10-05（D-08 第四部分：快捷键录制框）

## 1. 任务状态

| 编号 | 任务 | 状态 | 代码 / PR | 还差什么 |
|---|---|---|---|---|
| D-01 | 前端骨架：Vite + Vue 3、设计令牌、类型生成封装、多窗口 | 完成 | `xinqing_hub/src/`：`styles/tokens.css`、`api/`、`i18n/`、`windows/shared/mount.ts`、`vite.config.ts` | DS-ICON 要求的 Lucide 图标库还没引入（第一个用到图标的界面再加，记得在“关于”页列许可） |
| D-02 | 小组件（布局、小精灵、一句话区、底栏、交互、贴边） | **进行中** | 第一部分（窗口行为、右键菜单）：[xinqing-ime#14](https://github.com/young8290/xinqing-ime/pull/14)（已合并）；第二部分（悬停解释）：[xinqing-ime#16](https://github.com/young8290/xinqing-ime/pull/16)；第三部分（全屏自动隐藏、首帧位置）、第四部分（“准 / 不准”）：随后的 PR | 见第 3 节 |
| D-03 | 小精灵插画与 lottie 动画（6 天气 + 3 一次性动作）、logo | **进行中** | `components/WeatherSprite.vue`：SVG + CSS 动画——每种天气 2–4 秒循环（呼吸、眨眼，加上光芒转、云飘、雨落、闪电每 3 秒闪一下、月亮浮、风线摆），一次性动作 `play('shake' \| 'approach')`，“闭眼”即 `eyesClosed`；`WeatherStage.vue` 交叉淡化。应用图标是几何占位（`tools/gen_hub_icons.py`，形状已按 DS-BRAND-02） | 正式插画与 lottie 素材（要设计稿，Claude 做不了插画定稿）；“晃一下 / 靠近”还没接上触发源：打错字要 B / A 给瞬时事件，暖心话要 C-04 的事件；logo 的矢量版已做（`components/AppLogo.vue`，与 `gen_hub_icons.py` 同一套几何），引导页还没用上 |
| D-04 | 卡片层（日程 / 待办 / 提醒 / 自评 / 小结 / 周信等 8 类） | 未开始 | — | 07 第 2 节的“卡片层”窗口还没在 `tauri.conf.json` 登记；卡片的数据来自 B、C 的事件，事件形状要先定（改 `bindings.ts` 走契约 PR） |
| D-05 | 首次引导与同意 | 大部分完成 | `windows/onboarding/`（FR-ONB-01～04，有测试） | FR-ONB-05（AI 地址与密钥、“测试连接”、小组件位置、关怀频率、晴晴打招呼）未做，“测试连接”依赖 C 的网关（xinqing-ime#7） |
| D-06 | 对话窗口（流式、标识、求助卡片 UI） | 第一版完成 | `windows/chat/`：`useChat.ts`（会话、流式事件、先到事件暂存）、`App.vue`（AI 说明、历史抽屉、气泡与 `AI 生成` 标签、复制、停止 / 重试）、`SafetyCard.vue`（求助卡片，折叠成一行不可移除）；与 C-07/C-08 同一个 PR，ADR 0018 | 快捷指令、记忆、历史搜索（P1）；窗口置顶切换与全局快捷键 `Ctrl+Alt+Q`；学校心理中心电话等设置项 |
| D-07 | 情绪看板 | 占位 | `windows/dashboard/`（只显示当前天气） | 全部（P1，计划 W9）；ECharts 未引入 |
| D-08 | 设置中心（schema 表单生成器 + 9 个分类） | **进行中** | `windows/settings/`：左侧分类 + 每个分类一个组件；“输入法”（xinqing-ime#36 已合并：常用项中文名称 + 其余按 schema 生成、分组放进“高级”，修改即保存、重启横幅）、“外观”、“隐私与关于”里的“关于”。契约（#35 已合并）：ADR 0016 + `core/src/infra/imeconf.rs`（wind-rpc 客户端，只用标准库）+ 命令 `ime_schema` / `ime_config_get` / `ime_config_set`。契约（xinqing-ime#38 已合并）：ADR 0017 + 事件通道读线程 `src-tauri/src/ime_events.rs` + 事件 `ime_config:changed`；“输入法”页接这个事件和窗口焦点刷新（xinqing-ime#39 已合并）；本 PR：快捷键录制框、中英切换键改为五选一 | 下一步：其余分类（FR-SET-03～08、10）按各自后端逐个补；“隐私”部分随 E 的 FR-DAT 任务；开源许可的完整依赖清单要用工具生成 |
| D-09 | 系统通知、无障碍与高对比度、DPI 走查 | 部分 | 令牌里已有 `forced-colors` 高对比度和 `prefers-reduced-motion`；不可见时暂停动画（`mount.ts` + `base.css`） | 系统通知（FR-NTF-01）、150% 文本大小与 100%–200% DPI 走查都没做 |

## 2. 代码地图（D 负责的部分）

```
xinqing_hub/
├─ src/styles/tokens.css        设计令牌（07 第 1 节），深浅色、高对比度
├─ src/styles/base.css          公共样式：焦点环、减少动画、不可见暂停动画
├─ src/api/                     bindings.ts（生成，D 是契约负责人）+ unwrap / CommandError
├─ src/i18n/                    zh-CN.ts（由 hub_templates/ui_copy.toml 生成）+ t / errorText
├─ src/stores/                  status（状态快照 + status:changed）、settings（设置缓存 + settings:changed）
├─ src/components/              AppLogo（logo 矢量版）、WeatherSprite（静态小精灵）、WeatherStage（交叉淡化）、ExplainPanel（状态解释，`actions` 插槽放“准 / 不准”）
├─ src/weather.ts               天气图标、名称、不确定说法
├─ src/selfReport.ts            自评选项文案、覆盖期推算（看板时间线也会用）
├─ src/explain.ts               状态解释的拼句（state_explain 的结构 → 标题 / 说明 / 来源），看板时间线也用它
├─ src/windows/<label>/         每个窗口一个入口：widget / chat / dashboard / settings / onboarding
│  └─ widget/
│     ├─ App.vue                小组件界面与交互
│     ├─ statusLine.ts          状态行（特殊情形优先级）
│     ├─ menu.ts                右键菜单项（规格顺序）
│     ├─ useExplain.ts          悬停 / 聚焦状态行时取解释、开关面板
│     ├─ useSelfReport.ts       自评：开关面板、提交、跟踪“你说的”覆盖期
│     ├─ SelfReportPanel.vue    “我现在…”面板
│     ├─ placement.ts           位置计算纯函数：默认位置、吸附、按显示器记住、小标签
│     └─ useWidgetWindow.ts     调 Tauri 窗口接口：恢复位置、拖动后吸附、贴边隐藏、置顶
│  └─ settings/
│     ├─ App.vue                左侧分类（FR-SET-01 的顺序），默认“输入法”，#about 直接打开“隐私与关于”
│     ├─ ImeSection.vue         输入法：取 schema 和配置、修改即保存、跳过原因 / 重启横幅 / 核心没运行；
│     │                         收到 ime_config:changed、窗口获得焦点、保存后都重取配置（合并重复的刷新）
│     ├─ ImeFieldControl.vue    一项配置按类型出控件（开关 / 下拉 / 数字 / 文本 / 列表；map、array 只读）
│     ├─ imeForm.ts             常用项清单（改时同步 ui_copy.toml 的 [ime.field]）、高级分组、按点分键名取值
│     ├─ hotkey.ts              快捷键：录制规则、键名与别名（hotkey.test.ts 对照清风 hotkey.rs）、用录制框的配置键清单
│     ├─ HotkeyRecorder.vue     快捷键录制框：点一下录制，Esc 取消、退格清除，重复时提示
│     ├─ AppearanceSection.vue  外观
│     ├─ AboutSection.vue       隐私与关于（目前只有“关于”）
│     └─ licenses.ts            开源许可条目；新增随包分发的组件 / 字体 / 素材 / 图标库时在这里加
├─ core/src/infra/imeconf.rs   wind-rpc 客户端（读写输入法配置，ADR 0016）
├─ src-tauri/src/commands/ime.rs  ime_schema / ime_config_get / ime_config_set
├─ src-tauri/src/ime_events.rs  常驻线程读 wind-rpc 事件通道，转成 ime_config:changed（ADR 0017）
└─ src-tauri/
   ├─ capabilities/              default.json（各窗口共用）、widget.json（只给小组件：挪动、缩放、显示、置顶）
   ├─ src/fullscreen.rs          前台全屏时自动隐藏小组件（判断逻辑 AutoHide 平台无关，探测只在 Windows）
   └─ src/windows.rs             建窗口；小组件建好先不显示，由前端定位后 show()
```

验证（在 `xinqing_hub/`）：`pnpm lint && pnpm test && pnpm build`；改了 `ui_copy.toml` 后 `pnpm gen:i18n`，并在仓库根目录跑
`python3 tools/check_templates.py hub_templates`（禁用词）。看界面：`pnpm dev` 后浏览器打开 `http://localhost:1420/widget/index.html`
（没有后端，窗口接口会失败并打警告，界面照常显示）；真窗口行为要 `pnpm tauri dev`（Linux 需要 WebKitGTK）。

## 3. 进行中：D-02

| 部分 | 需求 | 状态 |
|---|---|---|
| 布局、状态行（暂停 / 未连接 / 冷启动优先级）、离线角标、单击打开对话、拖动 | FR-WGT-02/03/06 | 完成（骨架阶段） |
| 默认位置（主显示器右下角 16 px）、拖到边缘 24 px 内吸附、按显示器记住位置、显示器拔掉回默认位置 | FR-WGT-01 | 完成（本 PR）。位置存在 WebView 的 localStorage（`xq.widget.placement`），只是本机界面偏好 |
| 贴边隐藏：吸附在左右边缘、开了 `widget.autohide` 时，鼠标离开 3 秒收成 24 × 72 小标签，移入或聚焦展开 | FR-WGT-01 | 完成（本 PR） |
| 置顶跟随 `widget.topmost`、透明度跟随 `widget.opacity` | FR-WGT-01 | 完成（本 PR） |
| 右键菜单（右键 / `Shift+F10` / 菜单键）：暂停·恢复感知、打开看板、设置、隐藏小组件 | FR-WGT-06 | 完成（本 PR）。其余项见下一行 |
| 菜单里的开始 / 结束专注、换装、待确认日程与待办 | FR-WGT-06 | 未做：分别等 B-08 专注、FR-REV-04（P2）、C-05 待确认列表；按 `menu.ts` 顶部注释的顺序插入（“我现在…”已在第五部分加上） |
| 天气切换 400 ms 交叉淡化 | FR-WGT-03 | 完成（本 PR） |
| 一句话区悬停显示全文 | FR-WGT-04 | 完成（本 PR） |
| 一句话区的消息优先级队列（求助 > 自评回应 > 卡片 > 休息提醒 > 暖心话 > 小结 > 今日一句 > 空闲问候） | FR-WGT-04 | 未做：消息来源的事件还没有（C-04 暖心话等），事件形状定了再写，避免先猜契约 |
| 悬停状态行显示解释 | FR-WGT-06、FR-STA-09 | 完成（第二部分）：鼠标停留 300 ms 或键盘聚焦状态行时，面板盖住右侧整列，标题（不确定说法 + 可能性）、每条说明一行、来源与冷启动附注弱化；`Esc`、失焦、移开、状态变化都会收起。每次打开都重新取 `state_explain(null)`；它和当前快照不是同一个状态时不用，退回只显示标题。可能性百分比从状态行的悬停提示挪进了面板 |
| 解释面板里的“准 / 不准” | FR-STA-07 | 完成（第四部分）：两个按钮放在来源那一行右侧，点了调 `submit_feedback('mood_state', null, …)`（记到最近一条状态记录，即当前显示的状态），换成“谢谢，我记下了”，状态变了再重新问。为了放下按钮，面板从只盖右侧一列改为盖住整个内容区（约 286 × 134 px），小精灵会被暂时盖住；按钮仍守 32 × 32 点击目标。文案在 `ui_copy.toml` 的 `[widget.feedback]` |
| 状态行 ✎ 与自评面板、自评后 60 分钟显示“你说的：…” | FR-WGT-02/03/06、FR-STA-10 | 完成（第五部分）：状态行右侧 ✎ 或右键菜单第一项“我现在…”打开面板（`SelfReportPanel.vue`，盖住内容区）——六个选项点一下就提交，备注选填（≤ 50 字，提示只存本地）；收到 `self_report:changed` 后状态行显示“🌙 你说的：有点累”，到期由本地定时器收尾；窗口打开时没有覆盖期快照，就由 `self_report_list(今天)` 的最后一条推出（`src/selfReport.ts`）。覆盖期内不弹解释面板。离线角标改为状态行内联，与 ✎ 并排 |
| 底栏：今日输入时长、下一个日程 / 待办数、专注倒计时 | FR-WGT-05 | 未做：等 B-08（使用时长）、C-05（日程待办） |
| 前台全屏应用时自动隐藏，退出后恢复 | FR-WGT-01 | 完成（第三部分）：外壳每秒调一次 `SHQueryUserNotificationState`（全屏应用、全屏 D3D 游戏、演示模式都算），进全屏时把可见的小组件藏起来，退出时只恢复自己藏的；用户自己隐藏的不会被叫回来，全屏期间用户叫出来的也不再藏回去 |
| 首次启动不在默认位置闪一下 | FR-WGT-01 | 完成（第三部分）：小组件配置成 `visible: false`，外壳建好后不显示，前端挪到记住的位置再 `show()`；定位失败也照样显示 |
| “晃一下”（打错字） | FR-WGT-03、DS-MOTION-02 | 未做：`status:changed` 不带 `typo`（它不是显示状态），需要一个瞬时事件，属于契约改动 |
| `Ctrl+Alt+W` 显示 / 隐藏 | FR-WGT-01、FR-ENT-04 | 未做：要引入 Tauri 全局快捷键插件；FR-ENT-04 的其他 Hub 快捷键（`Ctrl+Alt+Q` / `D`）一起做，注册失败要在设置页提示冲突 |

## 4. 关键决定与待评审

- **右键菜单用系统原生菜单**（`@tauri-apps/api/menu` 的 `Menu.popup`），不在网页里画：小组件窗口只有 320 × 168，菜单项补齐后有 8 项，网页菜单画不下；原生菜单还自带讲述人与高对比度支持（DS-A11Y-02/04）。代价是菜单外观不走设计令牌。07 没规定菜单样式，不算偏离产品书，未写 ADR。
- **小组件位置存在 WebView 的 localStorage**，没有进 Hub 数据库：它是本机界面偏好，不是用户数据，也不需要随导出导入走；放进 `settings` 表要新增设置键（C 的契约）。以后要让“删除全部数据”也清掉它，再挪进数据库。
- **ADR 0016（输入法设置的读写与表单生成，D 提议）**：心晴不带清风的设置程序（托盘“设置”打开的是 Hub），而 wind-rpc 的 `config.schema` 只有键名、类型和枚举值、没有中文名称和范围，所以“输入法”分类改为常用项手写名称、其余按 schema 放进“高级”；客户端不依赖清风的 crate，协议版本号由测试核对。待 A、C 评审，接受后改产品书 07 FR-SET-02、10 第 5.1 节、17 第 3.4 节。
- **ADR 0017（输入法配置变更事件，D 提议）**：外壳常驻一个线程连 `xinqing_rpc{后缀}_events`（后缀在 `_events` 之前，和控制通道相反），把 `config.changed` 转成 `ime_config:changed {reason, needs_restart}`；每次连上先推一条 `connected`，核心没运行时 1～30 秒退避重连。核心自己的语言栏 / 菜单写配置不广播，所以设置窗口获得焦点时也要重新取。待 A、C 评审，接受后改产品书 10 第 5.2 节、07 FR-SET-02。
- **ADR 0012（AI 服务配置，C 提议）D 的评审意见：同意**（写在 ADR 的“评审”一节）。E 要求 release 只收 `https://`，落地时 `error.ai_config_invalid` 文案要同改；建议 `ai_config_get` 带上两侧健康状态，FR-SET-08 页面要用。
- **ADR 0010（状态解释的信号挑选与拼句）D 的评审意见：同意**（B 在 xinqing-ime#11 请 D 看界面拼句）。结构化的
  `Explanation { state, prob, signals[{kind, value}], source, cold_start }` 够界面用：`kind` 的序列化名就是
  `explain.signal.<kind>.text` 的键，`value` 统一传给 `{p}` / `{n}` / `{m}` 即可（`t()` 不认识的变量原样保留）。
  三点不阻塞的建议：① 小组件悬停时界面会排成多行（不确定说法 + 可能性、每条说明一行、来源和冷启动附注用 `--xq-text-3`），
  `note.format` 那种单行格式只给回放和评测用，建议 ADR 第 5 条写明“界面可以按自己的版式排”；② `StatusSnapshot.prob`
  是 0–1 小数，`Explanation.prob` 是 0–100 整数，同一个量两种单位，加 `state_explain` 命令时最好统一（或至少在字段名上区分）；
  ③ 实时解释最好带上它对应的窗口或时间，界面拿到后能确认它和当前快照是同一次切换，避免悬停时显示上一个状态的解释。（界面暂时用“解释的状态 = 快照的状态”判断，同一状态两次切换之间分不出来。）
- **小标签 24 × 72**：07 只规定了宽 24 px，高度是这里定的（够露出 20 px 的小精灵）。24 px 宽低于 DS-A11Y-03 的 32 px 点击目标，但小标签不需要点击——鼠标移入或键盘聚焦就展开，所以没有按 32 px 做。如果评审认为要守 32 px，07 的 24 px 也要一起改。

## 5. 已知问题

- `status.paused` 同时表示“用户暂停”和“无痕 / 密码框 / 黑名单”，菜单在密码框里也会显示“恢复感知”，点了不会解除闸门；要区分需要后端在快照里分开两个字段（B / A）。
- 吸附靠“最后一次移动事件后 400 ms 没再动”判断拖动停下。拖着不松手停顿超过 400 ms，窗口会先吸附一次，继续拖时 Windows 会把它拉回鼠标处，只是一下跳动。Tauri 没有“拖动结束”事件，暂时这样。
- 全屏检测只在进出全屏的那一刻动手：如果小组件恰好是在全屏期间才建出来的（例如在全屏游戏里完成了引导），它会一直显示到下次进出全屏。
- 自评覆盖期跨过午夜时（23:30 自评、00:10 重开小组件），当天的 `self_report_list` 里没有那一条，重开后不再显示“你说的”；要彻底解决得让快照带上覆盖期（B 的契约）。
- 负面自评后晴晴的回应和“和晴晴聊聊”按钮（FR-STA-10 第 2 条）要等 C-04 暖心话的事件，界面这边还没接。
- `AppState::db_rebuilt`（数据库损坏后已重建）还没有在一句话区提示（文案 `error.db_rebuilt`），等消息优先级队列一起做。
- 快捷键录制框在 Hub 窗口里录：输入法自己的快捷键（例如 `Ctrl+Shift+E` 轮换方案）可能先被核心吃掉，录不到；`Win` 组合多数被系统占用。都要在 Windows 真机上走一遍。
- “输入法”分类：wind-rpc 的 schema 没有数值范围和说明文字，高级区只显示键名；方案名来自 ui_copy 的 `[ime.schema]`，核心新增方案时显示原 id，需要补文案。没在真机上连过清风核心。
- 窗口行为（吸附、贴边隐藏、多显示器）只有纯函数单测和组件测试，还没在 Windows 真机上走查（100%–200% 混合缩放、任务栏在左 / 上）。

## 6. 下一步（按优先级）

1. D-03 剩余：“晃一下 / 靠近”等事件到位后接上（`play()` 已备好）；引导页用上 logo；
2. D-05：FR-ONB-05，等 C 的网关 PR（xinqing-ime#7）合并后接“测试连接”；
3. D-08：其余分类（FR-SET-03～08、10）按各自后端补；其余分类按各自后端补；
4. D-04：卡片层窗口与事件形状（契约 PR），先做休息提醒卡片（配合 B-08）。

## 7. 修订记录

| 日期 | 改动 |
|---|---|
| 2026-10-04 | 初版：盘点 D-01～D-09 现状；D-02 第一部分（小组件窗口行为、右键菜单、交叉淡化） |
| 2026-10-04 | D-02 第二部分：悬停状态行显示解释（接 B 的 `state_explain`） |
| 2026-10-04 | D-02 第三部分：前台全屏时自动隐藏小组件、首帧不闪 |
| 2026-10-04 | D-02 第四部分：解释面板里的“准 / 不准”（接 B 的 `submit_feedback`） |
| 2026-10-04 | D-03：小精灵 SVG + CSS 循环动画与一次性动作（lottie 前的占位） |
| 2026-10-04 | 修 `Explanation.prob` → `prob_pct` 改名后 main 上的类型错误（xinqing-ime#23）；设置中心加左侧分类与“关于”、logo 矢量版 |
| 2026-10-04 | D-02 第五部分：“我现在…”自评面板、状态行“你说的”（接 B 的 `self_report_*`） |
| 2026-10-05 | D-08 契约：ADR 0016、wind-rpc 客户端与 `ime_*` 三个命令 |
| 2026-10-05 | D-08 第二部分：设置中心“输入法”分类（常用项 + 高级区，修改即保存） |
| 2026-10-05 | D-08 契约：ADR 0017、wind-rpc 事件通道读线程与 `ime_config:changed` 事件 |
| 2026-10-05 | D-08 第三部分：“输入法”页接 `ime_config:changed` 与窗口焦点刷新 |
| 2026-10-05 | D-06 第一版：对话窗口（随 C-07/C-08，ADR 0018） |
| 2026-10-05 | D-08 第四部分：快捷键录制框（键名对照清风 hotkey.rs）、中英切换键五选一、快捷键中文名称 |
