# xinqing_hub（Tauri 外壳 + Vue 前端）

- 职责：Hub 的界面与接口层（17 第 2、3 节）。`src-tauri/` 是 Tauri 外壳：单实例、启动参数、窗口管理、命令与事件；领域逻辑全部在 [`core/`](core/README.md)（ADR 0007）。`src/` 是 Vue 3 前端，每个窗口一个入口。
- 窗口：`widget`（小组件）、`chat`（对话）、`dashboard`（看板）、`settings`（设置）、`onboarding`（首次引导），尺寸与样式见 `src-tauri/tauri.conf.json`，与 07 第 2 节窗口清单一致；全部按需创建。
- 命令与事件：已实现 `get_status`、`state_explain`、`submit_feedback`、`self_report_set`、`self_report_list`、`pause_set`、`settings_get`、`settings_set`、`consent_get`、`consent_set`、`open_window`、`ai_config_get`、`secrets_set`、`ai_test_connection`、`ai_usage_today`，事件 `status:changed`、`settings:changed`、`self_report:changed`、`gateway:health`（10 第 5 节）。其余命令随各功能任务补上；以 `src-tauri/src/lib.rs` 的 `specta_builder` 为准。
- 生成的文件（不要手改，CI 会核对）：
  - `src/api/bindings.ts`：命令与事件的 TypeScript 封装，`pnpm gen:bindings`（即 `cargo run -p xinqing-hub --bin export-bindings`）；
  - `src/i18n/zh-CN.ts`：界面文案表，`pnpm gen:i18n`，来源是 `hub_templates/ui_copy.toml` 和 `explain.toml`。窗口标题也要和 `ui_copy.toml` 的 `window.*` 一致。
- 连接输入法：启动后作为 XQP 客户端连接核心（10 第 2 节），Windows 上是命名管道 `xinqing_tap`（调试构建 `xinqing_tap_dev`）；设置环境变量 `XQ_XQP_TCP=127.0.0.1:18765` 时改连 `xq-sim --tcp`，任何平台都能这样联调。出厂模板默认从可执行文件旁的 `hub_templates/` 读取，调试构建回退到仓库根目录，也可用 `XQ_HUB_TEMPLATES` 指定。
- AI 服务：外壳托管 `Arc<Ai>`（`src-tauri/src/gateway.rs`，本身实现 `AiGateway`）。配置来源按优先级：调试构建的仓库根目录 `secrets.toml`（或 `XQ_SECRETS_FILE` 指定）→ 设置页保存的 `secrets.bin`（DPAPI，只在 Windows）→ 调试构建连本机 mock-ai（`XQ_JEV_BASE_URL` / `XQ_LLM_BASE_URL` 可改）→ 离线（ADR 0012）。
- 数据目录：`%LOCALAPPDATA%\XinQing\hub\`（调试构建为 `XinQingDev`）；设置环境变量 `XQ_HUB_DATA_DIR` 可指到别处，演示和测试时不碰真实数据。
- 图标：`src-tauri/icons/` 是几何占位（`python3 tools/gen_hub_icons.py` 生成），正式设计稿到位后直接覆盖。

## 开发

需要 Node.js 22 LTS、pnpm 9（`corepack enable`）、Rust stable；Linux 上另需 WebKitGTK（`libwebkit2gtk-4.1-dev` 等，见 CI）。

```bash
pnpm install
pnpm tauri dev            # 热重载开发；首次运行会打开引导窗口
pnpm lint                 # ESLint + Prettier + vue-tsc
pnpm test                 # Vitest（单次运行；边改边测用 pnpm vitest）
pnpm tauri build --no-bundle   # 只出 xinqing_hub.exe，不打安装包（03 第 7 节）
```

不装输入法也能看到状态变化：先启动模拟器，再带环境变量启动 Hub（Windows 上去掉 `--tcp` 和环境变量即走命名管道）：

```bash
cargo run -p xq-sim -- --script tools/xq-sim/scripts/hesitant.jsonl --speed 5 --tcp 127.0.0.1:18765
XQ_XQP_TCP=127.0.0.1:18765 pnpm tauri dev
```

只看界面不需要后端时，`pnpm dev` 后在浏览器打开 `http://localhost:1420/widget/index.html` 等页面（命令调用会失败，界面按默认值显示）。

## 约定

- 前端只显示后端给的状态，不自行推断（17 第 3.2 节）；窗口打开时取快照，之后只跟随事件。
- 固定文案只能来自 `ui_copy.toml`，组件里写裸字符串会被 ESLint（`vue/no-bare-strings-in-template`）拦下。
- 颜色、字号、间距只用 `src/styles/tokens.css` 里的 `--xq-*` 令牌（07 第 1 节）。
