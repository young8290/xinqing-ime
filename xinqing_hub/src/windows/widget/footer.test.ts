import { describe, expect, it } from 'vitest'
import type { ScheduleItem } from '@/api'
import { footerLeft, footerRight, nextSchedule } from './footer'

const NOW = new Date(2026, 9, 9, 14, 0).getTime()

function item(p: Partial<ScheduleItem>): ScheduleItem {
  return {
    id: 1,
    title: '组会',
    date: '2026-10-09',
    time: '15:00',
    end_time: null,
    all_day: false,
    location: null,
    is_deadline: false,
    remind_offsets: [],
    status: 'added',
    source: 'ai',
    flags: [],
    created_ts: 0,
    ...p,
  }
}

describe('底栏（FR-WGT-05）', () => {
  it('左边是今日输入时长，写成“1 小时 42 分”', () => {
    expect(footerLeft(102)).toBe('⌨ 今日 1 小时 42 分')
    expect(footerLeft(0)).toBe('⌨ 今日 0 分钟')
    expect(footerLeft(120)).toBe('⌨ 今日 2 小时')
  })

  it('右边是未来 24 小时内最近的已添加日程', () => {
    const items = [
      item({ id: 1, title: '晚饭', time: '18:00' }),
      item({ id: 2, title: '组会', time: '15:00' }),
      item({ id: 3, title: '已开始', time: '13:00' }),
      item({ id: 4, title: '待确认', time: '14:30', status: 'pending' }),
      item({ id: 5, title: '后天', date: '2026-10-11', time: '09:00' }),
    ]
    const next = nextSchedule(items, NOW)
    expect(next?.item.id).toBe(2)
    expect(footerRight(next, 3, NOW)).toBe('📅 15:00 组会')
  })

  it('明天的日程标“明天”；今天的全天日程排最前', () => {
    const tomorrow = nextSchedule([item({ date: '2026-10-10', time: '09:00', title: '早课' })], NOW)
    expect(footerRight(tomorrow, 0, NOW)).toBe('📅 明天 09:00 早课')
    const allDay = nextSchedule(
      [item({ title: '组会' }), item({ id: 9, title: '运动会', time: null, all_day: true })],
      NOW,
    )
    expect(footerRight(allDay, 0, NOW)).toBe('📅 全天 运动会')
  })

  it('没有日程时显示待办数，都没有就不显示', () => {
    expect(footerRight(null, 3, NOW)).toBe('✅ 3 件待办')
    expect(footerRight(null, 0, NOW)).toBeNull()
  })
})
