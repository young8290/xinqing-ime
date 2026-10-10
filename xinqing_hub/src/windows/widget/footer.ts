// 小组件底栏（07 FR-WGT-05）：左边今日输入时长；右边未来 24 小时内最近的一个日程，没有日程时显示未完成待办数，
// 都没有就不显示。专注倒计时随专注计时（FR-RST-10，P2）再加。纯函数，取数在 useFooter.ts。
import type { ScheduleItem } from '@/api'
import { duration, localDate, parseDate } from '@/format'
import { t } from '@/i18n'

const DAY_MS = 24 * 60 * 60 * 1000

export interface Upcoming {
  item: ScheduleItem
  /** 开始时刻；全天或没有时刻的日程是那天 0 点 */
  start: number
  allDay: boolean
}

/** 日程的开始时刻（本地时间）；没有日期时为 null。 */
function startOf(s: ScheduleItem): { start: number; allDay: boolean } | null {
  const day = s.date ? parseDate(s.date) : null
  if (!day) return null
  const allDay = s.all_day || !s.time
  if (!allDay) {
    const [h, m] = s.time!.split(':').map(Number)
    day.setHours(h ?? 0, m ?? 0, 0, 0)
  }
  return { start: day.getTime(), allDay }
}

/**
 * 未来 24 小时内最近的一个已添加日程。今天的全天日程也算（排在最前），已经开始的有时刻日程不算。
 */
export function nextSchedule(items: readonly ScheduleItem[], now: number): Upcoming | null {
  const today = localDate(new Date(now))
  let best: (Upcoming & { key: number }) | null = null
  for (const item of items) {
    if (item.status !== 'added' || !item.title) continue
    const s = startOf(item)
    if (!s) continue
    const todayAllDay = s.allDay && item.date === today
    if (!todayAllDay && (s.start < now || s.start > now + DAY_MS)) continue
    const key = todayAllDay ? now : s.start
    if (!best || key < best.key) best = { item, ...s, key }
  }
  return best && { item: best.item, start: best.start, allDay: best.allDay }
}

/** 底栏右边的文字；没有日程也没有待办时为 null。 */
export function footerRight(next: Upcoming | null, openTodos: number, now: number): string | null {
  if (next) {
    const title = next.item.title ?? ''
    const time = next.allDay ? t('widget.footer.all_day') : (next.item.time ?? '')
    const tomorrow = next.item.date !== localDate(new Date(now))
    return t(tomorrow ? 'widget.footer.next_tomorrow' : 'widget.footer.next', { time, title })
  }
  return openTodos > 0 ? t('widget.footer.todos', { n: openTodos }) : null
}

/** 底栏左边：今日输入时长（FR-RST-01 的活跃分钟） */
export function footerLeft(typingMin: number): string {
  return t('widget.footer.typing', { duration: duration(typingMin) })
}
