// 看板 · 情绪日历（07 FR-DSH-03）的纯函数：月视图网格（周一在前）、月份切换、格子的读屏名称。
// 每天的主导天气由后端按 FR-DSH-03 算好（month_moods），打字太少的天为空，显示“—”。
import type { DayMood } from '@/api'
import { dayLabel, localDate } from '@/format'
import { t } from '@/i18n'
import { STATE_WEATHER, WEATHER_ICON, weatherName } from '@/weather'

export interface CalendarCell {
  date: string
  day: number
  /** 主导天气图标；没有为 null（显示“—”） */
  icon: string | null
  hasDiary: boolean
  today: boolean
  /** 格子按钮的读屏名称：“10月9日 周五，多云，有日记” */
  label: string
}

/** `YYYY-MM` 这个月往前或往后 n 个月 */
export function shiftMonth(month: string, n: number): string {
  const [y, m] = month.split('-').map(Number)
  const d = new Date(y!, (m ?? 1) - 1 + n, 1)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}`
}

export function monthOf(date: Date): string {
  return localDate(date).slice(0, 7)
}

export function monthLabel(month: string): string {
  const [y, m] = month.split('-').map(Number)
  return t('dashboard.calendar.month', { y: y!, m: m! })
}

/** 月视图：按周分行，周一在前；月初之前、月末之后的位置为 null */
export function monthGrid(
  month: string,
  moods: readonly DayMood[],
  today: string,
): (CalendarCell | null)[][] {
  const [y, m] = month.split('-').map(Number)
  const first = new Date(y!, (m ?? 1) - 1, 1)
  const days = new Date(y!, m ?? 1, 0).getDate()
  const lead = (first.getDay() + 6) % 7
  const byDate = new Map(moods.map((d) => [d.date, d]))
  const cells: (CalendarCell | null)[] = Array.from({ length: lead }, () => null)
  for (let day = 1; day <= days; day++) {
    const date = `${month}-${String(day).padStart(2, '0')}`
    const mood = byDate.get(date)
    const w = mood?.dominant ? STATE_WEATHER[mood.dominant] : null
    const hasDiary = mood?.has_diary ?? false
    const label =
      t('dashboard.calendar.day_label', {
        date: dayLabel(date),
        weather: w ? weatherName(w) : t('dashboard.none'),
      }) + (hasDiary ? t('dashboard.calendar.day_diary') : '')
    cells.push({ date, day, icon: w ? WEATHER_ICON[w] : null, hasDiary, today: date === today, label })
  }
  while (cells.length % 7 !== 0) cells.push(null)
  const weeks: (CalendarCell | null)[][] = []
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7))
  return weeks
}

const WEEKDAY = new Intl.DateTimeFormat('zh-CN', { weekday: 'narrow' })

/** 表头：一 二 三 四 五 六 日（2026-10-05 是周一） */
export const WEEK_HEAD: string[] = Array.from({ length: 7 }, (_, i) =>
  WEEKDAY.format(new Date(2026, 9, 5 + i)),
)
