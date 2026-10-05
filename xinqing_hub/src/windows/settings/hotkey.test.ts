import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  CODE_TO_KEY,
  HOTKEY_KEYS,
  KEY_ALIASES,
  TOGGLE_MODE_KEYS,
  formatHeld,
  formatHotkey,
  normalizeHotkey,
  recordKey,
} from './hotkey'

/** 清风的源码：键名表的唯一真相源，这里逐个对照，两边改名时测试失败（ADR 0016 第 5 条） */
// vitest 在 xinqing_hub/ 下运行（本地与 CI 都是 `pnpm test`）
const wind = (rel: string) => readFileSync(resolve(process.cwd(), '../wind_input/crates', rel), 'utf8')
const hotkeyRs = wind('wind-config/src/hotkey.rs')
const schemaRs = wind('wind-config/src/config_schema.rs')

/** 取 Rust 函数体（到下一个顶格的 `}` 为止） */
function fnBody(src: string, name: string): string {
  const start = src.indexOf(`fn ${name}(`)
  expect(start, `找不到 fn ${name}`).toBeGreaterThan(-1)
  return src.slice(start, src.indexOf('\n}\n', start))
}

/** `parse_key_name` 的 match 分支：键名 → 虚拟键码 */
function coreKeyNames(): Map<string, number> {
  const names = new Map<string, number>()
  const arm = /((?:"(?:[^"\\]|\\.)*"\s*\|\s*)*"(?:[^"\\]|\\.)*")\s*=>\s*Some\((0x[0-9A-Fa-f]+)\)/g
  for (const m of fnBody(hotkeyRs, 'parse_key_name').matchAll(arm)) {
    for (const lit of m[1]!.matchAll(/"((?:[^"\\]|\\.)*)"/g)) {
      names.set(lit[1]!.replace(/\\(.)/g, '$1'), Number(m[2]))
    }
  }
  return names
}

const core = coreKeyNames()
/** 核心认这个键名吗：表里有，或是单个字母（`parse_key_name` 最后那条分支） */
const coreKnows = (k: string) => core.has(k) || /^[a-z]$/.test(k)
const key = (code: string, mods: Partial<Record<'ctrl' | 'alt' | 'shift' | 'meta', boolean>> = {}) => ({
  code,
  ctrlKey: !!mods.ctrl,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
  metaKey: !!mods.meta,
})

describe('快捷键：与清风核心的键名逐个对照', () => {
  it('读到了 parse_key_name 的分支', () => {
    expect(core.get('space')).toBe(0x20)
    expect(core.get('\\')).toBe(0xdc)
    expect(core.get('equal')).toBe(0xbb)
  })

  it('录制时会写出的每个键名，核心都认', () => {
    for (const name of Object.values(CODE_TO_KEY)) expect(coreKnows(name), name).toBe(true)
  })

  it('别名表的每一组，在核心里指向同一个键', () => {
    for (const [alias, canon] of Object.entries(KEY_ALIASES)) {
      expect(core.get(alias), alias).toBeDefined()
      expect(core.get(alias), `${alias} → ${canon}`).toBe(core.get(canon))
    }
  })

  it('修饰键写法与 parse_hotkey 一致', () => {
    const body = fnBody(hotkeyRs, 'parse_hotkey')
    for (const arm of ['"ctrl" | "control"', '"alt"', '"shift"', '"win" | "super"']) {
      expect(body).toContain(arm)
    }
  })

  it('中英切换键的五个单键，compile_toggle_mode_key 都认', () => {
    const body = fnBody(hotkeyRs, 'compile_toggle_mode_key')
    for (const k of TOGGLE_MODE_KEYS) expect(body, k).toContain(`"${k}"`)
  })

  it('录制框负责的配置键都在注册表里，类型是字符串', () => {
    for (const k of HOTKEY_KEYS) expect(schemaRs, k).toContain(`f("${k}", Str)`)
  })
})

describe('快捷键录制', () => {
  it('组合键按固定顺序写出', () => {
    expect(recordKey(key('KeyE', { shift: true, ctrl: true }))).toEqual({
      kind: 'done',
      value: 'ctrl+shift+e',
    })
    expect(recordKey(key('BracketRight', { ctrl: true, alt: true }))).toEqual({
      kind: 'done',
      value: 'ctrl+alt+]',
    })
    expect(recordKey(key('Equal', { ctrl: true }))).toEqual({ kind: 'done', value: 'ctrl+equal' })
    expect(recordKey(key('Space', { shift: true }))).toEqual({ kind: 'done', value: 'shift+space' })
    expect(recordKey(key('F11'))).toEqual({ kind: 'done', value: 'f11' })
  })

  it('只按修饰键时继续等；Esc 取消；单按退格清除', () => {
    expect(recordKey(key('ControlLeft', { ctrl: true }))).toEqual({ kind: 'pending', held: 'ctrl' })
    expect(recordKey(key('Escape'))).toEqual({ kind: 'cancel' })
    expect(recordKey(key('Backspace'))).toEqual({ kind: 'clear' })
    expect(recordKey(key('Backspace', { ctrl: true }))).toEqual({
      kind: 'done',
      value: 'ctrl+backspace',
    })
  })

  it('会和打字冲突的不收；核心不认的键不收', () => {
    for (const k of [
      key('KeyA'),
      key('KeyA', { shift: true }),
      key('Space'),
      key('Digit1', { shift: true }),
    ]) {
      expect(recordKey(k), k.code).toEqual({ kind: 'invalid', reason: 'need_modifier' })
    }
    expect(recordKey(key('Numpad1', { ctrl: true }))).toEqual({ kind: 'invalid', reason: 'unsupported' })
  })

  it('规范写法：大小写、顺序、别名统一；不绑返回 null', () => {
    expect(normalizeHotkey('Shift+Ctrl+E')).toBe('ctrl+shift+e')
    expect(normalizeHotkey('control+=')).toBe('ctrl+equal')
    expect(normalizeHotkey('ctrl+period')).toBe('ctrl+.')
    expect(normalizeHotkey('none')).toBeNull()
    expect(normalizeHotkey('')).toBeNull()
    expect(normalizeHotkey('ctrl+a+b')).toBeNull()
    expect(normalizeHotkey(7)).toBeNull()
  })

  it('显示：Ctrl + Shift + E；不绑显示“未设置”', () => {
    expect(formatHotkey('ctrl+shift+e')).toBe('Ctrl + Shift + E')
    expect(formatHotkey('ctrl+shift+\\')).toBe('Ctrl + Shift + \\')
    expect(formatHotkey('ctrl+equal')).toBe('Ctrl + =')
    expect(formatHotkey('none')).toBe('未设置')
    expect(formatHotkey(undefined)).toBe('未设置')
    expect(formatHeld('ctrl+shift')).toBe('Ctrl + Shift + …')
    expect(formatHeld('')).toBe('')
  })
})
