#!/usr/bin/env node
// 从 hub_templates 生成前端文案表 src/i18n/zh-CN.ts（17 第 3.1 节：文案单一来源）。
//
//   pnpm gen:i18n            重新生成
//   pnpm gen:i18n --check    只核对，不一致时退出码 1（CI 用）
//
// 收录 ui_copy.toml 的全部字符串（键名不加前缀，与 UiError.message_key 一致）和
// explain.toml 的全部字符串（加 `explain.` 前缀）。同时核对 tauri.conf.json 的窗口标题与 `window.*` 一致。
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { parse } from 'smol-toml'

const hub = fileURLToPath(new URL('..', import.meta.url))
const templates = `${hub}/../hub_templates`
const out = `${hub}/src/i18n/zh-CN.ts`
const SKIP = new Set(['version', 'exempt'])

function* strings(table, prefix) {
  for (const [k, v] of Object.entries(table)) {
    if (!prefix && SKIP.has(k)) continue
    const key = prefix ? `${prefix}.${k}` : k
    if (typeof v === 'string') yield [key, v]
    else if (v && typeof v === 'object' && !Array.isArray(v)) yield* strings(v, key)
  }
}

const load = (name) => parse(readFileSync(`${templates}/${name}`, 'utf8'))
const entries = [...strings(load('ui_copy.toml'), ''), ...strings(load('explain.toml'), 'explain')]
const copy = Object.fromEntries(entries)
if (Object.keys(copy).length !== entries.length) throw new Error('文案键重复')

const errors = []
const conf = JSON.parse(readFileSync(`${hub}/src-tauri/tauri.conf.json`, 'utf8'))
for (const w of conf.app.windows) {
  if (copy[`window.${w.label}`] !== w.title) {
    errors.push(`tauri.conf.json 窗口 ${w.label} 的标题“${w.title}”与 ui_copy.toml window.${w.label} 不一致`)
  }
}

const body = entries.map(([k, v]) => `  ${JSON.stringify(k)}: ${JSON.stringify(v)},`).join('\n')
const text = `// 本文件由 \`pnpm gen:i18n\` 从 hub_templates/ui_copy.toml 与 explain.toml 生成，请勿手改。
export const zhCN = {
${body}
} as const

export type CopyKey = keyof typeof zhCN
`

if (process.argv.includes('--check')) {
  let current = ''
  try {
    current = readFileSync(out, 'utf8')
  } catch {
    // 文件不存在按不一致处理
  }
  if (current !== text) errors.push('src/i18n/zh-CN.ts 已过期，请运行 pnpm gen:i18n')
} else {
  writeFileSync(out, text)
  console.log(`已生成 ${entries.length} 条文案到 src/i18n/zh-CN.ts`)
}

if (errors.length) {
  console.error(errors.join('\n'))
  process.exit(1)
}
