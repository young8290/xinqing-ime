# 心晴身份改造

产品书 14 §1 第 4 步（FR-OPS-06、C-PLT-08）：把 Windows 上会与官方清风输入法冲突的标识换成心晴自己的，
使两者能同时安装、互不干扰（TC-OPS-04）。

改造由脚本完成，规则写在 `scripts/xinqing/rebrand.py` 里，本文只说明改了什么、为什么、合并上游时怎么做。

## 合并上游后

```bash
git merge upstream/main
python3 scripts/xinqing/rebrand.py          # 把上游新代码里的标识改过来
python3 scripts/xinqing/rebrand.py --check  # CI 同样会跑这一步，有残留即失败
```

脚本是幂等的，可以反复运行。上游若新增了脚本规则覆盖不到的标识（例如新的内核对象名、新的 GUID），
`--check` 不一定能发现，合并时请顺手看一眼上游提交里新增的 `Local\`、`\\.\pipe\`、`CLSID`、`Software\`。

## 对照表

| 类别 | 清风（上游） | 心晴 | 位置 |
|---|---|---|---|
| TSF GUID（release） | `{99C2EE3n-5C57-45A2-9C63-FB54B34FD90A}` | `{EF62EE3n-5ECF-413A-A476-48D1F29E827C}` | `wind_tsf/src/Globals.cpp`、`direct_switch.rs`、`tsf_profile_name.rs`、`dev.ps1`、`ime-slot.ps1`、`config/app.toml` |
| TSF GUID（dev） | `{99C2DEBn-…}` | `{EF62DEBn-…}` | 同上 |
| 主管道 | `\\.\pipe\wind_input[_dev]_<SID>` | `\\.\pipe\xinqing[_dev]_<SID>` | `Globals.cpp`、`wind-bridge/src/server.rs` |
| 推送管道 | `wind_input_push[_dev]` | `xinqing_push[_dev]` | `Globals.cpp`、`wind-bridge/src/push.rs` |
| 设置端 RPC 管道 | `wind_input_ctrl`、`wind_input{后缀}_events` | `xinqing_rpc_ctrl`、`xinqing_rpc{后缀}_events` | `wind-rpc` |
| 数据目录 | `%LOCALAPPDATA%\WindInput[Dev]`、`%APPDATA%\WindInput[Dev]` | `XinQing[Dev]` | `wind-config/src/variant.rs`、`wind_tsf/include/FileLogger.h` |
| 注册表 | `HKLM\Software\WindInput[Dev]` | `HKLM\Software\XinQing[Dev]` | `Globals.h`（`WIND_APP_REGKEY`）、`tsf_profile_name.rs`、`dev.ps1` |
| 内核对象 | `Local\WindInput_*`、`Local\WindInputIMEService*` | `Local\XinQing_*`、`Local\XinQingIMEService*` | `wind-ipc`、`wind-bridge`、`apps/service`、`Globals.h` |
| 窗口类名 / 窗口消息 | `WindInputCandidate`、`WindInputHotkeyWnd`、`WindInputHotkeyRetry_v1` 等 | `XinQing…` | `wind-ui`、`wind_tsf/src` |
| 显示名 | 清风输入法（开发版） | 心晴输入法（开发版） | `Globals.h`、`LangBarItemButton.cpp`、`version.rc.in`、`build.rs`、协调器文案 |
| 安装目录 | `C:\Program Files\WindInput[Dev]` | `C:\Program Files\XinQing[Dev]` | `dev.ps1`、`config/app.toml` |
| 系统目录副本 | `System32\IME\WindInput` | `System32\IME\XinQing` | `config/app.toml` |
| 图标 | 清风图标 | 占位图标（暖色底白色“晴”字） | `wind_tsf/res/wind_input.ico`、`assets/installer.ico`、`assets/logo.png` |

窗口类名和窗口消息也要改，是因为两个输入法的 DLL 会被同一个宿主进程加载：
`TextService.cpp` 用 `FindWindowExW` 按类名找热键窗口，同名时会找到对方的窗口。

## 刻意没改的

- **可执行文件名**（`wind_input.exe`、`wind_tsf.dll`、`wind_setting.exe`）与 crate 名、代码标识符：
  改名牵动构建脚本、CI 和上游合并，收益只是“名字好看”。两边装在不同目录、用不同的 CLSID 和管道，
  同名不影响共存。代价是按镜像名杀进程会误伤对方，所以 `dev.ps1` 的
  `Stop-WindService` / `Stop-ProcessForFile` 改成只停可执行文件位于本次部署目录下的进程（`Get-InDirProcesses`）。
- **macOS / Linux 端**（`wind_macos/`、`wind_linux/`、`wind-bridge/src/endpoint.rs`、`mac_panel.rs` 等）：
  心晴只做 Windows（C-PLT-01），这些文件与 Swift 侧逐字对齐，单改一侧反而会断。
  注意 `variant.rs` 的数据目录名在 macOS 上也会变成 `XinQing`，与 `scripts/mac/` 不再一致，不影响 Windows。
- **日志与命令行文案**里的 “WindInput”（如 `eprintln!("WindInput: …")`、`用法: wind_input …`）：
  不参与任何标识，留给后续界面统一改名时处理。
- **词库测试里的“清风输入法”**：那是测试用的候选词，不是产品名。
- **测试里的假路径和测试输入**（`cli_util.rs` 的 `fake_resolve`、`handle_addword.rs` 的分词样例）：与真实标识无关，脚本里用 `APP_NAME_EXCLUDE` 跳过。

## 已知限制

- 安装包由独立仓库 `wind-installer` 生成，本仓没有它，`dev.ps1` 的打包命令（`8` / `d8`）暂时不可用。
  `config/app.toml` 的 `process_names` 仍是按镜像名，接入安装器时要改成按路径停进程，否则安装心晴会停掉官方清风。
- 图标是占位图，正式图标出稿后覆盖上面三个文件即可（或改 `scripts/xinqing/gen_icons.py` 重新生成）。
- 本仓私有、不公开发布（C-PRJ-04），`BRANDING.md` 对公开分支的三条请求暂不适用；
  若将来公开，需在 README 注明“本项目是清风输入法 (WindInput) 的非官方分支，与原项目及其作者无关”。

## 验证

Linux 上能做的：`cargo test`（核心逻辑与名称断言）和 `rebrand.py --check`。
真正的共存验证（TC-OPS-04）必须在 Windows 上做：先装官方清风，再用 `.\scripts\dev.ps1 p1` 部署心晴，确认

1. 输入法列表里同时出现“清风输入法”和“心晴输入法”，可分别切换、分别打字；
2. 部署或卸载心晴时，官方清风的 `wind_input.exe` 没有被停掉；
3. `%LOCALAPPDATA%\XinQing\` 与 `%LOCALAPPDATA%\WindInput\` 各自独立；
4. `scripts\ime-slot.ps1` 显示心晴占用的是 `EF62…` 槽位。
