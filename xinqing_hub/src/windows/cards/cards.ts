// 卡片上的文字（07 FR-WGT-07、06 FR-SCH-05/11/13、05 FR-REV-01）：日期写“10月9日 周五 15:00”（DS-COPY-07），
// 日程标记、时间重叠、晚间小结的几行统计。纯函数，组件里只管排版。
import type { ConflictItem, EveningSummary, ScheduleItem, TodoItem } from '@/api'
import { dayLabel, duration, shortDay } from '@/format'
import { t, isCopyKey } from '@/i18n'
import { STATE_WEATHER, WEATHER_ICON, weatherName } from '@/weather'

/** 日程的时间：“10月9日 周五 15:00”、全天、截止；没有日期时只写时刻 */
export function scheduleWhen(s: ScheduleItem): string {
  const day = s.date ? dayLabel(s.date) : ''
  let time = ''
  if (s.all_day || (!s.time && s.date)) time = t('cards.all_day')
  else if (s.time && s.is_deadline) time = t('cards.deadline', { time: s.time })
  else if (s.time) time = s.end_time ? `${s.time}–${s.end_time}` : s.time
  return [day, time].filter(Boolean).join(' ')
}

/** 卡片上的提示（`local` 请确认信息、`maybe_dup` 可能已经添加过……）；不认识的标记不显示 */
export function flagTexts(flags: readonly string[]): string[] {
  return flags
    .map((f) => `cards.flag.${f}`)
    .filter(isCopyKey)
    .map((k) => t(k))
}

/** “⚠ 与「班会」时间重叠（15:00–16:00）”（FR-SCH-11） */
export function conflictText(c: ConflictItem): string {
  const range = c.start === null ? t('cards.all_day') : c.end ? `${c.start}–${c.end}` : c.start
  return t('cards.conflict', { title: c.title, range })
}

/** 待办的截止：“截止 10月4日”；没有截止日期为 null */
export function todoDue(td: TodoItem): string | null {
  return td.due_date ? t('cards.todo_due', { date: shortDay(td.due_date) }) : null
}

/** 晚间小结的几行（FR-REV-01 第 2 条）：打字、喝水、休息；日程、待办 */
export function eveningLines(s: EveningSummary): string[][] {
  return [
    [
      t('cards.evening_typing', { duration: duration(s.typing_min) }),
      t('cards.evening_water', { n: s.water }),
      t('cards.evening_rest', { done: s.rests_done, due: s.rests_due }),
    ],
    [t('cards.evening_schedules', { n: s.schedules_done }), t('cards.evening_todos', { n: s.todos_done })],
  ]
}

/** 天气色带：图标串（☀️⛅☀️🌧☀️）和给读屏的说法（“今天的天气：晴、多云、晴”） */
export function eveningBand(s: EveningSummary): { icons: string; label: string } {
  const weathers = s.band.map((st) => STATE_WEATHER[st]).filter((w) => w !== null)
  return {
    icons: weathers.map((w) => WEATHER_ICON[w]).join(''),
    label: t('cards.evening_band', { band: weathers.map(weatherName).join('、') }),
  }
}
