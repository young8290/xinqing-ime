// 自评天气（04 FR-STA-10）的界面侧逻辑：选项文案、当前是否处在“你说的”覆盖期。
// 覆盖本身由后端维护（自评后 60 分钟，“说不上来”不覆盖）；窗口打开时没有快照可取，
// 就从当天的自评记录推出来，之后跟随 self_report:changed。看板时间线也会用到这里。
import type { SelfReportItem, SelfWeather } from '@/api'
import { t, type CopyKey } from '@/i18n'

/** 选项顺序按 FR-STA-10 */
export const SELF_WEATHERS: SelfWeather[] = ['sunny', 'cloudy', 'rain', 'storm', 'night', 'unsure']

/** 自评覆盖显示的时长（与后端 self_report::OVERRIDE_MS 一致） */
export const OVERRIDE_MS = 60 * 60 * 1000

/** 备注上限（FR-STA-10：≤ 50 字；后端也会截断） */
export const NOTE_MAX = 50

/** 选项按钮上的文字，带 emoji，例如“🌙 有点累” */
export function selfOption(w: SelfWeather): string {
  return t(`self_report.options.${w}` satisfies CopyKey)
}

/** 状态行里用的说法，去掉开头的 emoji，例如“有点累”（“你说的：有点累”） */
export function selfLabel(w: SelfWeather): string {
  const text = selfOption(w)
  const space = text.indexOf(' ')
  return space < 0 ? text : text.slice(space + 1)
}

export interface SelfOverride {
  weather: Exclude<SelfWeather, 'unsure'>
  /** Unix 毫秒 */
  until: number
}

/** 由一次自评算出覆盖期；“说不上来”或已过期时没有覆盖。 */
export function overrideFrom(weather: SelfWeather, until: number | null, now: number): SelfOverride | null {
  if (weather === 'unsure' || until === null || until <= now) return null
  return { weather, until }
}

/** 当天的自评记录（按时间先后）里最后一条决定现在是否还在覆盖期。 */
export function overrideFromList(items: SelfReportItem[], now: number): SelfOverride | null {
  const last = items.at(-1)
  if (!last || last.ts === null) return null
  return overrideFrom(last.weather, last.ts + OVERRIDE_MS, now)
}

/** 本地日期 YYYY-MM-DD（self_report_list 的参数） */
export function localDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`
}
