# Moodflow 插件市场索引仓库模板

插件市场就是一个 Git 仓库（照 Obsidian 的做法，见 [插件系统设计](../../docs/design/plugin-system.md) §7）。
本目录是这个仓库的**起始内容**：建好 `moodflow-plugins` 仓库后，把本目录的文件原样复制到它的根目录。

| 文件 | 用途 |
|---|---|
| `index.json` | 市场索引：插件、各版本的下载地址、`sha256`、大小、最低宿主版本、平台；撤回列表 |
| `index.json.sig` | 维护者对 `index.json` 原始字节的 Ed25519 签名（十六进制），客户端先验签再解析 |
| `.github/workflows/review.yml` | PR 审核：下载每个版本的包，用 `plugin_tool review` 跑与客户端安装同一套检查 |

## 上架 / 发版流程

1. 开发者在插件目录运行 `plugin_tool check <目录>`，再 `plugin_tool pack <目录> <输出.mfpack>`；
   `pack` 打印的 JSON（`version`、`sha256`、`size`、`min_host_version`、`platforms`）就是一条 release。
2. 把 `.mfpack` 发到自己仓库的 Release，向索引仓库提 PR：在 `plugins` 里新增条目或给已有条目追加 release
   （`url` 填 Release 资产的下载地址）。
3. CI 自动审核：哈希、身份（包内清单的 id/版本与索引一致）、清单合法、受保护文件、WASM 只导入允许的宿主函数
   且导入与声明的能力一致。L1（WASM）插件还要人工审源码；L0 内容包可只走自动检查。
4. 合入后维护者在**自己的机器上**签名并推送签名文件：

   ```sh
   MF_MARKET_SIGNING_KEY=<64 位十六进制种子> plugin_tool sign index.json   # 写出 index.json.sig
   ```

## 撤回

在 `revoked` 里加 `{"id": "...", "version": "1.2.0", "reason": "..."}`（省略 `version` 表示整个插件撤回），
重新签名。客户端下次同步时停用已安装的撤回版本并提示，更新时跳过撤回版本。

## 签名密钥（须线下完成，CI 与本仓都不保存）

- 生成：任意可信机器上生成 32 字节随机种子（例如 `openssl rand -hex 32`），离线保存两份。
- 公钥：运行一次 `plugin_tool sign`，它会打印公钥；把公钥写进客户端（`moodflow-market` 的调用方）作为内置常量。
- 私钥**绝不**进仓库、CI secrets 或聊天记录。换钥需要发新版客户端。
