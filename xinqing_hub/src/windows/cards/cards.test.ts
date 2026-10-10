import { describe, expect, it } from 'vitest'
import type { EveningSummary, ScheduleItem } from '@/api'
import { conflictText, eveningBand, eveningLines, flagTexts, scheduleWhen, todoDue } from './cards'
import { add, ordered, remove, update, visible, type Card } from './queue'

function sch(p: Partial<ScheduleItem> = {}): ScheduleItem {
  return {
    id: 1,
    title: '组会',
    date: '2026-10-09',
    time: '15:00',
    end_time: null,
    all_day: false,
    location: '实验楼',
    is_deadline: false,
    remind_offsets: [],
    status: 'pending',
    source: 'ai',
    flags: [],
    created_ts: 0,
    ...p,
  }
}

describe('卡片上的文字', () => {
  it('日程时间按 DS-COPY-07：10月9日 周五 15:00', () => {
    expect(scheduleWhen(sch())).toBe('10月9日 周五 15:00')
    expect(scheduleWhen(sch({ end_time: '16:00' }))).toBe('10月9日 周五 15:00–16:00')
    expect(scheduleWhen(sch({ time: null, all_day: true }))).toBe('10月9日 周五 全天')
    expect(scheduleWhen(sch({ time: '23:59', is_deadline: true }))).toBe('10月9日 周五 23:59 截止')
    expect(scheduleWhen(sch({ date: null }))).toBe('15:00')
  })

  it('标记只显示认识的；时间重叠写明是哪个、几点（FR-SCH-11）', () => {
    expect(flagTexts(['local', 'maybe_dup', 'whatever'])).toEqual(['请确认信息', '可能已经添加过'])
    expect(conflictText({ id: 2, title: '班会', start: '15:00', end: '16:00' })).toBe(
      '⚠ 与「班会」时间重叠（15:00–16:00）',
    )
    expect(conflictText({ id: 2, title: '运动会', start: null, end: null })).toBe(
      '⚠ 与「运动会」时间重叠（全天）',
    )
  })

  it('待办截止', () => {
    const td = {
      id: 1,
      title: '打印简历',
      due_date: '2026-10-04',
      status: 'pending',
      source: 'ai',
      created_ts: 0,
      done_ts: null,
    }
    expect(todoDue(td)).toBe('截止 10月4日')
    expect(todoDue({ ...td, due_date: null })).toBeNull()
  })

  it('晚间小结：统计行与天气色带（unknown 不画）', () => {
    const s: EveningSummary = {
      date: '2026-10-09',
      typing_min: 252,
      water: 3,
      rests_done: 5,
      rests_due: 7,
      schedules_done: 2,
      todos_done: 3,
      band: ['fluent', 'hesitant', 'unknown', 'low'],
      line: '今天辛苦了，晚上好好休息。',
    }
    expect(eveningLines(s)).toEqual([
      ['⌨ 打字 4 小时 12 分', '💧 喝水 3 次', '👀 休息 5/7 次'],
      ['📅 完成日程 2 个', '✅ 完成待办 3 个'],
    ])
    expect(eveningBand(s)).toEqual({ icons: '☀️⛅🌧', label: '今天的天气：晴、多云、小雨' })
  })
})

describe('卡片队列（FR-WGT-07）', () => {
  const rest = (seq: number): Card => ({ key: 'rest', kind: 'rest', due: { kind: 'eye', tired: seq > 1 } })
  const letter: Card = {
    key: 'letter:1',
    kind: 'letter',
    letter: { id: 1, week_start: '2026-10-05', ai_generated: true },
  }
  const reply: Card = { key: 'self_reply', kind: 'self_reply', comfortId: 3 }
  const detecting: Card = {
    key: 'schedule:7',
    kind: 'schedule',
    cardId: 7,
    item: null,
    conflicts: [],
    done: false,
  }

  it('最多显示 2 张，按优先级排，其余排队', () => {
    let q = add([], letter, 1)
    q = add(q, rest(2), 2)
    q = add(q, detecting, 3)
    const v = visible(q)
    expect(v.shown.map((c) => c.kind)).toEqual(['schedule', 'rest'])
    expect(v.waiting).toBe(1)
    q = add(q, reply, 4)
    expect(visible(q).shown.map((c) => c.kind)).toEqual(['self_reply', 'schedule'])
  })

  it('同一个 key 替换：休息提醒只留最新一张', () => {
    let q = add([], rest(1), 1)
    q = add(q, rest(2), 2)
    expect(ordered(q)).toHaveLength(1)
    expect(ordered(q)[0]).toMatchObject({ kind: 'rest', due: { tired: true } })
  })

  it('识别完成后填字段，收起后移出', () => {
    let q = add([], detecting, 1)
    q = update(q, 'schedule:7', (c) => (c.kind === 'schedule' ? { ...c, item: sch() } : c))
    expect(ordered(q)[0]).toMatchObject({ item: { title: '组会' } })
    expect(remove(q, 'schedule:7')).toEqual([])
    expect(update(q, 'nope', (c) => c)).toEqual(q)
  })
})
