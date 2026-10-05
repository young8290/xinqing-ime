// 设置中心“关怀”（07 FR-SET-05）的输入校验。规则与后端 `SettingItem::accepts`（core/src/domain/settings.rs、dnd.rs）一致：
// 后端对不合格的值只回“出了点小问题”，前端先拦下来，告诉用户具体哪里不对。后端仍是最终裁判。

/** 一个进程名最多多少字（`dnd::APP_MAX_CHARS`） */
export const APP_MAX_CHARS = 64
/** 电话号码最多多少字（`PHONE_MAX_CHARS`） */
export const PHONE_MAX_CHARS = 24
/** “晴晴记住的事”最多几件、每件最多几个字（FR-CHT-07 第 2 条，`chat::MEMORY_MAX` / `MEMORY_ENTRY_CHARS`） */
export const MEMORY_MAX = 50
export const MEMORY_ENTRY_CHARS = 100

const chars = (s: string) => [...s].length

/** 进程名：非空、不超过 64 字，不带路径分隔符和控制字符。调用前先去掉首尾空白。 */
export function validApp(s: string): boolean {
  // eslint-disable-next-line no-control-regex
  return s !== '' && s === s.trim() && chars(s) <= APP_MAX_CHARS && !/[\\/:\u0000-\u001f\u007f]/.test(s)
}

/** 列表里已经有了（忽略 ASCII 大小写，与后端去重规则一致）。 */
export function hasApp(list: readonly string[], app: string): boolean {
  const a = app.toLowerCase()
  return list.some((x) => x.toLowerCase() === a)
}

/** 电话号码：空串表示没填；否则只含数字、空格与 `+-()`，至少 3 个数字，不超过 24 字，首尾没有空白。 */
export function validPhone(s: string): boolean {
  if (s === '') return true
  return (
    chars(s) <= PHONE_MAX_CHARS &&
    s === s.trim() &&
    /^[\d +\-()]+$/.test(s) &&
    (s.match(/\d/g)?.length ?? 0) >= 3
  )
}

/** `<input type="time">` 的两个值拼成 `"HH:MM-HH:MM"`；缺一个或起止相同返回 null（后端不收）。 */
export function quietRange(from: string, to: string): string | null {
  const hm = /^\d{2}:\d{2}$/
  if (!hm.test(from) || !hm.test(to) || from === to) return null
  return `${from}-${to}`
}

/** 显示用：`"22:00-07:00"` → `"22:00–07:00"` */
export function showRange(r: string): string {
  return r.replace('-', '–')
}

/** 跨午夜（结束早于开始）的时段 */
export function crossesMidnight(r: string): boolean {
  const [a, b] = r.split('-')
  return a !== undefined && b !== undefined && b < a
}

/** 要记住的事：去掉首尾空白后非空且不超过 100 字（`chat::memory_entry`）。 */
export function validMemory(s: string): boolean {
  const t = s.trim()
  return t !== '' && chars(t) <= MEMORY_ENTRY_CHARS
}
