// “输入法”分类的表单数据（07 FR-SET-02、ADR 0016）：常用项清单、高级区分组、按点分键名取值、文案。
// wind-rpc 的 config.schema 只有键名、类型和枚举值：常用项在这里配中文名称，其余按 schema 放进“高级”，名称就是键名。
import type { ImeField } from '@/api'
import { isCopyKey, t } from '@/i18n'

/**
 * 常用项（07 FR-SET-02 点名的输入方案、候选个数、中英切换键、主题，加两项高频外观），按显示顺序。
 * 文案键为 `ime.field.<键名把点换成下划线>`。改这里时同步 hub_templates/ui_copy.toml 的 [ime.field]。
 */
export const COMMON_KEYS = [
  'schema.active',
  'ui.candidate.per_page',
  'ui.candidate.layout',
  'keys.toggle_mode_keys',
  'ui.theme.name',
  'ui.candidate.font_size',
] as const

const copyId = (key: string) => key.replaceAll('.', '_')

/** 控件旁的名称：常用项用中文，其余直接显示键名。 */
export function fieldLabel(key: string): string {
  const k = `ime.field.${copyId(key)}`
  return isCopyKey(k) ? t(k) : key
}

/** 枚举 / 方案取值的显示名：有文案用文案，没有就显示原值。 */
export function optionLabel(key: string, value: string): string {
  const k = key === 'schema.active' ? `ime.schema.${value}` : `ime.option.${copyId(key)}.${value}`
  return isCopyKey(k) ? t(k) : value
}

/** 按点分键名从整份配置里取值；路径不存在时返回 undefined。 */
export function getPath(values: unknown, key: string): unknown {
  let cur: unknown = values
  for (const part of key.split('.')) {
    if (cur === null || typeof cur !== 'object' || !Object.hasOwn(cur, part)) return undefined
    cur = (cur as Record<string, unknown>)[part]
  }
  return cur
}

/**
 * `schema.active` 在 schema 里是自由字符串，但合法值就是 `schema.available`：有这张表时当下拉框用。
 * 其余字段原样返回。
 */
export function effectiveField(f: ImeField, values: unknown): ImeField {
  if (f.key !== 'schema.active') return f
  const available = getPath(values, 'schema.available')
  if (!Array.isArray(available) || !available.every((v) => typeof v === 'string')) return f
  return { ...f, kind: 'enum', options: available }
}

export interface AdvancedGroup {
  /** 键名第一段，例如 `input` */
  prefix: string
  fields: ImeField[]
}

/** 高级区：除常用项以外的全部字段，按键名第一段分组、组内按键名排序。 */
export function advancedGroups(fields: ImeField[]): AdvancedGroup[] {
  const common = new Set<string>(COMMON_KEYS)
  const groups = new Map<string, ImeField[]>()
  for (const f of fields) {
    if (common.has(f.key)) continue
    const prefix = f.key.split('.')[0] ?? f.key
    groups.set(prefix, [...(groups.get(prefix) ?? []), f])
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([prefix, fs]) => ({ prefix, fields: fs.sort((a, b) => a.key.localeCompare(b.key)) }))
}

/** 常用项按清单顺序取出；schema 里没有的（核心版本不同）就跳过。 */
export function commonFields(fields: ImeField[]): ImeField[] {
  const byKey = new Map(fields.map((f) => [f.key, f]))
  return COMMON_KEYS.flatMap((k) => byKey.get(k) ?? [])
}

/** 数字输入框的值转回数字：空串或不是数时返回 null（不提交）。整数项取整。 */
export function parseNumber(raw: string, kind: 'int' | 'float'): number | null {
  if (raw.trim() === '') return null
  const n = Number(raw)
  if (!Number.isFinite(n)) return null
  return kind === 'int' ? Math.round(n) : n
}
