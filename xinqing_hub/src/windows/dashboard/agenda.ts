// 看板 · 日程与待办（06 FR-SCH-09、FR-SCH-13）的纯函数：已添加的日程分“即将”与“已过期”，
// 表单与 ScheduleInput / TodoInput 互转，按后端同样的规则先校验（标题 12 / 16 字、HH:MM、全天不带时刻），说清哪里不对。
import type { ScheduleInput, ScheduleItem, TodoInput, TodoItem } from '@/api'
import type { CopyKey } from '@/i18n'

/** 标题上限（与后端 check_schedule / check_todo 一致） */
export const SCHEDULE_TITLE_MAX = 12
export const TODO_TITLE_MAX = 16
/** 提醒的选项（分钟），与设置 `sch.default_offsets` 同一组；`default` 是按设置 */
export const REMIND_CHOICES = ['default', '0', '5', '10', '30', '60', '1440'] as const
export type RemindChoice = (typeof REMIND_CHOICES)[number]

/** 日程开始的排序键：`YYYY-MM-DD HH:MM`，全天按 00:00，没有日期的排最后 */
function sortKey(s: ScheduleItem): string {
  return s.date ? `${s.date} ${s.all_day ? '00:00' : (s.time ?? '00:00')}` : '9999'
}

/**
 * 已添加的日程分成还没过的与已过期的（ADR 0032：后端不写 expired，界面自己分）：
 * 日期在今天之前，或今天且（结束或开始）时刻已过的算过期；没有日期的一直算“即将”。
 */
export function splitAdded(
  items: readonly ScheduleItem[],
  now: Date,
): { upcoming: ScheduleItem[]; expired: ScheduleItem[] } {
  const pad = (n: number) => String(n).padStart(2, '0')
  const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
  const hhmm = `${pad(now.getHours())}:${pad(now.getMinutes())}`
  const upcoming: ScheduleItem[] = []
  const expired: ScheduleItem[] = []
  for (const s of items) {
    const end = s.end_time ?? s.time
    const past = !!s.date && (s.date < today || (s.date === today && !s.all_day && !!end && end < hhmm))
    ;(past ? expired : upcoming).push(s)
  }
  upcoming.sort((a, b) => sortKey(a).localeCompare(sortKey(b)))
  expired.sort((a, b) => sortKey(b).localeCompare(sortKey(a)))
  return { upcoming, expired }
}

export interface ScheduleForm {
  title: string
  date: string
  time: string
  end_time: string
  all_day: boolean
  location: string
  is_deadline: boolean
  remind: RemindChoice
}

export function emptySchedule(date: string): ScheduleForm {
  return {
    title: '',
    date,
    time: '',
    end_time: '',
    all_day: false,
    location: '',
    is_deadline: false,
    remind: 'default',
  }
}

export function scheduleForm(s: ScheduleItem): ScheduleForm {
  const first = s.remind_offsets.length === 1 ? String(Math.round(s.remind_offsets[0]! / 60)) : null
  const remind = (REMIND_CHOICES as readonly string[]).includes(first ?? '')
    ? (first as RemindChoice)
    : 'default'
  return {
    title: s.title ?? '',
    date: s.date ?? '',
    time: s.time ?? '',
    end_time: s.end_time ?? '',
    all_day: s.all_day,
    location: s.location ?? '',
    is_deadline: s.is_deadline,
    remind,
  }
}

/** 字数按“字”算，不按 UTF-16 码元 */
const chars = (s: string) => [...s].length

export type Checked<T> = { ok: true; input: T } | { ok: false; error: CopyKey; vars?: Record<string, number> }

export function scheduleInput(f: ScheduleForm): Checked<ScheduleInput> {
  const title = f.title.trim()
  if (!title) return { ok: false, error: 'dashboard.schedule.title_required' }
  if (chars(title) > SCHEDULE_TITLE_MAX)
    return { ok: false, error: 'dashboard.schedule.title_too_long', vars: { n: SCHEDULE_TITLE_MAX } }
  const time = f.all_day ? null : f.time || null
  const end = f.all_day || f.is_deadline ? null : f.end_time || null
  if (time && !f.date) return { ok: false, error: 'dashboard.schedule.time_needs_date' }
  if (time && end && end <= time) return { ok: false, error: 'dashboard.schedule.end_before_start' }
  return {
    ok: true,
    input: {
      title,
      date: f.date || null,
      time,
      end_time: end,
      all_day: f.all_day,
      location: f.location.trim() || null,
      is_deadline: f.is_deadline,
      // 全天日程固定当天 09:00 提醒（后端规则），不给选
      remind_offsets: f.remind === 'default' || f.all_day ? null : [Number(f.remind) * 60],
    },
  }
}

export interface TodoForm {
  title: string
  due_date: string
}

export function todoForm(td: TodoItem | null): TodoForm {
  return { title: td?.title ?? '', due_date: td?.due_date ?? '' }
}

export function todoInput(f: TodoForm): Checked<TodoInput> {
  const title = f.title.trim()
  if (!title) return { ok: false, error: 'dashboard.schedule.title_required' }
  if (chars(title) > TODO_TITLE_MAX)
    return { ok: false, error: 'dashboard.schedule.title_too_long', vars: { n: TODO_TITLE_MAX } }
  return { ok: true, input: { title, due_date: f.due_date || null } }
}

/** 看板导航带来的 `edit`：日程是行号，待办带 `todo:` 前缀 */
export function parseEdit(edit: string | undefined): { kind: 'schedule' | 'todo'; id: number } | null {
  if (!edit) return null
  const todo = edit.startsWith('todo:')
  const id = Number(todo ? edit.slice(5) : edit)
  return Number.isInteger(id) && id > 0 ? { kind: todo ? 'todo' : 'schedule', id } : null
}
