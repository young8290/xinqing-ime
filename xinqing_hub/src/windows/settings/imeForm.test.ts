import { describe, expect, it } from 'vitest'
import type { ImeField } from '@/api'
import {
  COMMON_KEYS,
  advancedGroups,
  commonFields,
  effectiveField,
  fieldLabel,
  getPath,
  optionLabel,
  parseNumber,
} from './imeForm'

const f = (key: string, kind: ImeField['kind'] = 'string', options: string[] | null = null): ImeField => ({
  key,
  kind,
  options,
})

describe('输入法表单', () => {
  it('常用项都有中文名称，其余显示键名', () => {
    for (const k of COMMON_KEYS) expect(fieldLabel(k), k).not.toBe(k)
    expect(fieldLabel('ui.candidate.per_page')).toBe('候选个数')
    expect(fieldLabel('input.emoji.enabled')).toBe('input.emoji.enabled')
  })

  it('取值显示名：方案和候选排列有中文，没有文案的显示原值', () => {
    expect(optionLabel('schema.active', 'wubi86')).toBe('五笔 86')
    expect(optionLabel('schema.active', 'my_schema')).toBe('my_schema')
    expect(optionLabel('ui.candidate.layout', 'vertical')).toBe('竖排')
    expect(optionLabel('ui.candidate.shadow', 'off')).toBe('off')
  })

  it('按点分键名取值，路径不存在返回 undefined', () => {
    const v = { ui: { candidate: { per_page: 7 } }, keys: { toggle_mode_keys: ['lshift'] } }
    expect(getPath(v, 'ui.candidate.per_page')).toBe(7)
    expect(getPath(v, 'keys.toggle_mode_keys')).toEqual(['lshift'])
    expect(getPath(v, 'ui.nope.x')).toBeUndefined()
    expect(getPath(null, 'a')).toBeUndefined()
  })

  it('输入方案用 schema.available 当下拉框的取值；没有这张表时原样', () => {
    const active = f('schema.active')
    const values = { schema: { available: ['pinyin', 'wubi86'] } }
    expect(effectiveField(active, values)).toEqual({
      key: 'schema.active',
      kind: 'enum',
      options: ['pinyin', 'wubi86'],
    })
    expect(effectiveField(active, {})).toBe(active)
    const other = f('ui.theme.name')
    expect(effectiveField(other, values)).toBe(other)
  })

  it('常用项按清单顺序，核心没有的跳过；高级区排除常用项、按第一段分组排序', () => {
    const fields = [
      f('ui.candidate.layout', 'enum', ['horizontal', 'vertical']),
      f('input.emoji.enabled', 'bool'),
      f('schema.active'),
      f('debug.log_level', 'enum', ['info', 'debug']),
      f('input.auto_pair.chinese', 'bool'),
    ]
    expect(commonFields(fields).map((x) => x.key)).toEqual(['schema.active', 'ui.candidate.layout'])
    expect(advancedGroups(fields).map((g) => [g.prefix, g.fields.map((x) => x.key)])).toEqual([
      ['debug', ['debug.log_level']],
      ['input', ['input.auto_pair.chinese', 'input.emoji.enabled']],
    ])
  })

  it('数字输入：空或非数不提交，整数项取整', () => {
    expect(parseNumber('', 'int')).toBeNull()
    expect(parseNumber('abc', 'float')).toBeNull()
    expect(parseNumber('7.6', 'int')).toBe(8)
    expect(parseNumber('14.5', 'float')).toBe(14.5)
  })
})
