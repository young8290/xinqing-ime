// 数字与时间的写法（07 DS-COPY-07）：时刻用 24 小时制 `15:00`，日期写“10月9日 周五”，时长写“1 小时 42 分”。
// 小组件、卡片层、看板共用，免得各处写出不一样的格式。
import { t } from '@/i18n'

const pad = (n: number) => String(n).padStart(2, '0')

/** 分钟数 → “1 小时 42 分”“45 分钟”“2 小时” */
export function duration(min: number): string {
  const m = Math.max(0, Math.round(min))
  const h = Math.floor(m / 60)
  const r = m % 60
  if (h === 0) return t('common.duration_m', { m: r })
  if (r === 0) return t('common.duration_h', { h })
  return t('common.duration_hm', { h, m: r })
}

/** Unix 毫秒 → 本地时刻“15:00” */
export function clockOf(ts: number): string {
  const d = new Date(ts)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

/** 本地日期 `YYYY-MM-DD` → Date（当天 0 点）。格式不对时为 null。 */
export function parseDate(date: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date)
  if (!m) return null
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]))
  return Number.isNaN(d.getTime()) ? null : d
}

const WEEKDAY = new Intl.DateTimeFormat('zh-CN', { weekday: 'short' })

/** `YYYY-MM-DD` → “10月9日 周五”；格式不对时原样返回 */
export function dayLabel(date: string): string {
  const d = parseDate(date)
  if (!d) return date
  return `${d.getMonth() + 1}月${d.getDate()}日 ${WEEKDAY.format(d)}`
}

/** `YYYY-MM-DD` → “10月9日”（不带星期，放不下时用） */
export function shortDay(date: string): string {
  const d = parseDate(date)
  return d ? `${d.getMonth() + 1}月${d.getDate()}日` : date
}

/** 本地日期 `YYYY-MM-DD` */
export function localDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/** 本地日期加减天数 */
export function addDays(date: string, n: number): string {
  const d = parseDate(date) ?? new Date()
  d.setDate(d.getDate() + n)
  return localDate(d)
}

/** 那周的周一（周一是一周的第一天，与周信、周报一致） */
export function weekStart(date: string): string {
  const d = parseDate(date) ?? new Date()
  const back = (d.getDay() + 6) % 7
  d.setDate(d.getDate() - back)
  return localDate(d)
}
