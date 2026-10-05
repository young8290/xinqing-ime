// 快捷键录制（07 FR-SET-02 “按键 → 快捷键录制框”、ADR 0016 第 5 条）。
// 核心认的写法是 `ctrl+shift+e` 这样的字符串，由清风 `wind-config/src/hotkey.rs` 的 `parse_hotkey` / `parse_key_name` 解析；
// wind-rpc 没有给出键名表的方法，所以这里留一份录制时会写出的键名，由 hotkey.test.ts 逐个对照那个文件，两边改名时测试失败。
import { t } from '@/i18n'

/** 用录制框编辑的配置键：值是单个组合键，`none` 或空串表示不绑（核心解析不出就当没绑）。 */
export const HOTKEY_KEYS = [
  'keys.switch_engine',
  'keys.toggle_full_width',
  'keys.toggle_punct',
  'keys.toggle_toolbar',
  'keys.open_settings',
  'keys.open_dictionary',
  'keys.add_word',
  'keys.open_add_word_dialog',
  'keys.toggle_s2t',
  'keys.toggle_t2s',
  'keys.activate_ime',
  'keys.take_screenshot',
  'keys.softkeyboard',
  'xinqing.pause_hotkey',
  'xinqing.rewrite_hotkey',
] as const

export function isHotkeyKey(key: string): boolean {
  return (HOTKEY_KEYS as readonly string[]).includes(key)
}

/** 不绑时写入的值（核心配置文件的注释用的也是它） */
export const UNSET = 'none'

/** 中英切换键（`keys.toggle_mode_keys`）只认这五个单键（`compile_toggle_mode_key`），按显示顺序。 */
export const TOGGLE_MODE_KEYS = ['lshift', 'rshift', 'lctrl', 'rctrl', 'capslock'] as const

/** 修饰键在组合里的固定顺序与写法（`parse_hotkey` 认 ctrl / alt / shift / win） */
const MODIFIERS = ['ctrl', 'alt', 'shift', 'win'] as const
type Modifier = (typeof MODIFIERS)[number]

/** `KeyboardEvent.code` → 核心的键名。没列出的键（小键盘、媒体键……）核心不认，录不了。 */
export const CODE_TO_KEY: Readonly<Record<string, string>> = {
  ...Object.fromEntries('abcdefghijklmnopqrstuvwxyz'.split('').map((c) => [`Key${c.toUpperCase()}`, c])),
  ...Object.fromEntries('0123456789'.split('').map((d) => [`Digit${d}`, d])),
  ...Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`F${i + 1}`, `f${i + 1}`])),
  Space: 'space',
  Enter: 'enter',
  Tab: 'tab',
  Backspace: 'backspace',
  Delete: 'delete',
  Insert: 'insert',
  Home: 'home',
  End: 'end',
  PageUp: 'pageup',
  PageDown: 'pagedown',
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
  Period: '.',
  Comma: ',',
  Semicolon: ';',
  Quote: "'",
  Slash: '/',
  Backslash: '\\',
  BracketLeft: '[',
  BracketRight: ']',
  // `-` `=` 用单词：和分隔符 `+` 写在一起时好认，核心出厂配置也这么写（`ctrl+equal`）
  Minus: 'minus',
  Equal: 'equal',
  Backquote: '`',
}

/**
 * 同一个键在核心里的其他写法 → 这里用的写法，用来判断两个快捷键是不是同一个（重复检测）。
 * 只收 `parse_key_name` 里一键多名的那几行；hotkey.test.ts 核对每组在核心里确实指向同一个键。
 */
export const KEY_ALIASES: Readonly<Record<string, string>> = {
  return: 'enter',
  escape: 'esc',
  back: 'backspace',
  del: 'delete',
  ins: 'insert',
  pgup: 'pageup',
  pgdn: 'pagedown',
  period: '.',
  comma: ',',
  semicolon: ';',
  quote: "'",
  slash: '/',
  backslash: '\\',
  lbracket: '[',
  rbracket: ']',
  '-': 'minus',
  hyphen: 'minus',
  '=': 'equal',
  equals: 'equal',
  backtick: '`',
  grave: '`',
}

const MODIFIER_ALIASES: Readonly<Record<string, Modifier>> = {
  ctrl: 'ctrl',
  control: 'ctrl',
  alt: 'alt',
  shift: 'shift',
  win: 'win',
  super: 'win',
}

/** 单独按下时会打出字符的键：只配这些键、或只加 Shift，会和正常打字冲突 */
function isPrintable(key: string): boolean {
  return key.length === 1 || key === 'minus' || key === 'equal' || key === 'space'
}

export type RecordResult =
  /** 只按了修饰键，继续等主键；`held` 用来实时显示 */
  | { kind: 'pending'; held: string }
  | { kind: 'done'; value: string }
  /** Esc：放弃这次录制 */
  | { kind: 'cancel' }
  /** 单按退格或 Delete：清除（不绑） */
  | { kind: 'clear' }
  | { kind: 'invalid'; reason: 'need_modifier' | 'unsupported' }

type KeyLike = Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>

const MODIFIER_CODES = new Set([
  'ShiftLeft',
  'ShiftRight',
  'ControlLeft',
  'ControlRight',
  'AltLeft',
  'AltRight',
  'MetaLeft',
  'MetaRight',
  'OSLeft',
  'OSRight',
])

/** 正按着的修饰键，按固定顺序。keyup 时事件上的标志已是松开之后的状态，可直接拿来刷新显示。 */
export function heldModifiers(e: Pick<KeyLike, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): Modifier[] {
  const on: Record<Modifier, boolean> = {
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    win: e.metaKey,
  }
  return MODIFIERS.filter((m) => on[m])
}

/** 录制框里的一次 keydown → 结果。 */
export function recordKey(e: KeyLike): RecordResult {
  const mods = heldModifiers(e)
  if (MODIFIER_CODES.has(e.code)) return { kind: 'pending', held: mods.join('+') }
  if (e.code === 'Escape') return { kind: 'cancel' }
  if (mods.length === 0 && (e.code === 'Backspace' || e.code === 'Delete')) return { kind: 'clear' }
  const key = CODE_TO_KEY[e.code]
  if (!key) return { kind: 'invalid', reason: 'unsupported' }
  // 功能键可以单按；会打出字符的键至少要带 Ctrl / Alt / Win（只加 Shift 就是大写字母或符号）
  const typing = mods.every((m) => m === 'shift') && isPrintable(key)
  const bare = mods.length === 0 && !/^f\d+$/.test(key)
  if (key === 'space' ? mods.length === 0 : typing || bare) {
    return { kind: 'invalid', reason: 'need_modifier' }
  }
  return { kind: 'done', value: [...mods, key].join('+') }
}

/** 规范写法：小写、修饰键按固定顺序、别名换成这里的写法；不绑或认不出返回 null。 */
export function normalizeHotkey(raw: unknown): string | null {
  if (typeof raw !== 'string') return null
  const parts = raw
    .split('+')
    .map((p) => p.trim().toLowerCase())
    .filter((p) => p !== '')
  if (parts.length === 0 || parts.includes(UNSET)) return null
  const mods = new Set<Modifier>()
  const keys: string[] = []
  for (const p of parts) {
    const m = MODIFIER_ALIASES[p]
    if (m) mods.add(m)
    else keys.push(KEY_ALIASES[p] ?? p)
  }
  if (keys.length !== 1) return null
  return [...MODIFIERS.filter((m) => mods.has(m)), keys[0]].join('+')
}

const KEY_DISPLAY: Readonly<Record<string, string>> = {
  ctrl: 'Ctrl',
  alt: 'Alt',
  shift: 'Shift',
  win: 'Win',
  space: 'Space',
  enter: 'Enter',
  esc: 'Esc',
  tab: 'Tab',
  backspace: 'Backspace',
  delete: 'Delete',
  insert: 'Insert',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  up: '↑',
  down: '↓',
  left: '←',
  right: '→',
  minus: '-',
  equal: '=',
}

function displayPart(p: string): string {
  return KEY_DISPLAY[p] ?? p.toUpperCase()
}

/** 显示用：`ctrl+shift+e` → `Ctrl + Shift + E`；不绑显示“未设置”；认不出的原样显示。 */
export function formatHotkey(raw: unknown): string {
  if (raw === undefined || raw === null || raw === '' || raw === UNSET) return t('ime.hotkey.unset')
  const norm = normalizeHotkey(raw)
  if (norm === null) return String(raw)
  return norm.split('+').map(displayPart).join(' + ')
}

/** 录制中按住的修饰键：`ctrl+shift` → `Ctrl + Shift + …` */
export function formatHeld(held: string): string {
  return held === '' ? '' : `${held.split('+').map(displayPart).join(' + ')} + …`
}
