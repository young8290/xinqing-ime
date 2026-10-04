// 文案取用：键来自生成的 zh-CN.ts（单一来源 hub_templates/ui_copy.toml），不在组件里手写固定文案。
import { zhCN, type CopyKey } from './zh-CN'

export type { CopyKey }

export type CopyVars = Record<string, string | number>

export function isCopyKey(key: string): key is CopyKey {
  return Object.hasOwn(zhCN, key)
}

/** 取文案并替换 `{变量}`；没给值的变量原样保留，便于发现漏传。 */
export function t(key: CopyKey, vars?: CopyVars): string {
  const text: string = zhCN[key]
  if (!vars) return text
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    Object.hasOwn(vars, name) ? String(vars[name]) : whole,
  )
}

/** 命令错误 → 给用户看的一句话（DS-COPY-06）。未知的文案键回落到通用提示，不显示错误码。 */
export function errorText(err: unknown): string {
  const key = typeof err === 'object' && err !== null && 'message_key' in err ? String(err.message_key) : ''
  return t(isCopyKey(key) ? key : 'error.generic')
}
