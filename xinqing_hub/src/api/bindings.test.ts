import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// ADR 0027：不会是 NaN 的 f64（时间戳等）要标 `#[specta(type = specta_typescript::Number)]`，导出成 `number`；
// 只有真的可能为空的 `Option` 字段才是 `number | null`。时间戳字段按名字守住：新加的 `ts` / `*_ts` 忘了标注会在这里失败。
const bindings = readFileSync(resolve(process.cwd(), 'src/api/bindings.ts'), 'utf8')

/** 真的可能为空的时间戳（Rust 里是 `Option<f64>`）：`stop_ts` 这一晚没有打字时为空，`done_ts` 待办还没完成时为空 */
const NULLABLE_TS = new Set(['stop_ts', 'done_ts'])

describe('前端绑定的数字类型（ADR 0027）', () => {
  it('时间戳字段导出成 number，只有清单里的可空', () => {
    const fields = [...bindings.matchAll(/^\s+(\w*_ts|ts): (number(?: \| null)?),$/gm)]
    expect(fields.length).toBeGreaterThan(5)
    for (const [, name, type] of fields) {
      expect(type, name).toBe(NULLABLE_TS.has(name!) ? 'number | null' : 'number')
    }
  })

  it('设置值没有 null（从 JSON 读出的数不会是 NaN）', () => {
    expect(bindings).toContain('export type SettingValue = boolean | number | string | string[];')
  })
})
