# xinqing_hub（Tauri 外壳 + Vue 前端）

- 职责：Hub 的界面与接口层（17 第 2、3 节）。`src-tauri/` 是 Tauri 外壳：单实例、启动参数、窗口管理、命令与事件；领域逻辑全部在 [`core/`](core/README.md)（ADR 0007）。`src/` 是 Vue 3 前端，每个窗口一个入口。
- 窗口：`widget`（小组件）、`chat`（对话）、`dashboard`（看板）、`settings`（设置）、`onboarding`（首次引导），尺寸与样式见 `src-tauri/tauri.conf.json`，与 07 第 2 节窗口清单一致；全部按需创建。
- 命令与事件：已实现 `get_status`、`pause_set`、`settings_get`、`settings_set`、`consent_get`、`consent_set`、`open_window`，事件 `status:changed`、`settings:changed`（10 第 5 节）。其余命令随各功能任务补上。
- 生成的文件（不要手改，CI 会核对）：
  - `src/api/bindings.ts`：命令与事件的 TypeScript 封装，`pnpm gen:bindings`（即 `cargo run -p xinqing-hub --bin export-bindings`）；
  - `src/i18n/zh-CN.ts`：界面文案表，`pnpm gen:i18n`，来源是 `hub_templates/ui_copy.toml` 和 `explain.toml`。窗口标题也要和 `ui_copy.toml` 的 `window.*` 一致。
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

只看界面不需要后端时，`pnpm dev` 后在浏览器打开 `http://localhost:1420/widget/index.html` 等页面（命令调用会失败，界面按默认值显示）。

## 约定

- 前端只显示后端给的状态，不自行推断（17 第 3.2 节）；窗口打开时取快照，之后只跟随事件。
- 固定文案只能来自 `ui_copy.toml`，组件里写裸字符串会被 ESLint（`vue/no-bare-strings-in-template`）拦下。
- 颜色、字号、间距只用 `src/styles/tokens.css` 里的 `--xq-*` 令牌（07 第 1 节）。
