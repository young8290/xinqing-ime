import { describe, expect, it } from 'vitest'
import type { DayStats, Pulse, ScheduleItem, Segment, WeekReport } from '@/api'
import { FALLBACK_CHART_COLORS as C } from '@/components/chartTheme'
import { emptySchedule, parseEdit, scheduleForm, scheduleInput, splitAdded, todoInput } from './agenda'
import { monthGrid, monthLabel, shiftMonth, WEEK_HEAD } from './calendar'
import { backspaceRuns, band, dayStart, dots, ecgLabel, ecgOption, summaryCards, trim } from './today'
import { distLabel, distOption, distRows, heatOption, mix, typingOption, versus, weekRange } from './weekly'

const stats = (p: Partial<DayStats> = {}): DayStats => ({
  date: '2026-10-09',
  typing_min: 102,
  rests_due: 4,
  rests_done: 3,
  water: 1,
  comforts: 2,
  dominant: 'hesitant',
  timeline: [],
  ...p,
})

describe('今日（FR-DSH-02）', () => {
  it('四张概要卡片', () => {
    expect(summaryCards(stats()).map((c) => [c.label, c.value, c.detail])).toEqual([
      ['今日输入时长', '1 小时 42 分', null],
      ['主导天气', '⛅ 多云', null],
      ['暖心话', '2 次', null],
      ['休息完成率', '75%', '完成 3 / 4 次'],
    ])
    const empty = summaryCards(stats({ dominant: null, rests_due: 0, rests_done: 0 }))
    expect(empty[1]!.value).toBe('—')
    expect(empty[3]).toMatchObject({ value: '—', detail: '今天还没有休息提醒' })
  })

  it('时间线放到 0–24 时轴上，跨出当天的截掉；自评是圆点', () => {
    const start = dayStart('2026-10-09')
    const H = 3_600_000
    const segs: Segment[] = [
      { start_ts: start - H, end_ts: start + H, state: 'tired', mood_id: 1 },
      { start_ts: start + 12 * H, end_ts: start + 18 * H, state: 'fluent', mood_id: 2 },
      { start_ts: start + 25 * H, end_ts: start + 26 * H, state: 'low', mood_id: 3 },
    ]
    const b = band(segs, start)
    expect(b.map((x) => [Math.round(x.left), Math.round(x.width), x.weather])).toEqual([
      [0, 4, 'night'],
      [50, 25, 'sunny'],
    ])
    expect(b[1]!.label).toBe('12:00–18:00 晴')
    const d = dots([{ id: 7, ts: start + 6 * H, weather: 'rain', note: null, auto_state: null }], start)
    expect(d).toEqual([{ id: 7, left: 25, label: '06:00 你说：有点低落' }])
  })

  it('打字心电图：只看最近 60 秒，成串退格（≥ 3 次）标出来', () => {
    const p = (ts: number, bs = false, iki = 200): Pulse => ({ ts, iki_ms: iki, backspace: bs })
    const now = 100_000
    const pulses = [
      p(30_000),
      p(50_000),
      p(51_000, true),
      p(52_000, true),
      p(53_000, true),
      p(54_000, true, 6000),
      p(60_000),
      p(61_000, true),
    ]
    expect(trim(pulses, now).map((x) => x.ts)).toEqual([
      50_000, 51_000, 52_000, 53_000, 54_000, 60_000, 61_000,
    ])
    expect(backspaceRuns(pulses).map((x) => x.ts)).toEqual([51_000, 52_000, 53_000, 54_000])
    expect(ecgLabel(trim(pulses, now))).toBe('打字心电图：最近 60 秒按了 7 次键，最长停了 6.0 秒，退格 5 次')
    const o = ecgOption(trim(pulses, now), [{ ts: 55_000, weather: 'rain' }], now, C)
    const series = o.series as { type: string; data: number[][]; markLine?: { data: unknown[] } }[]
    expect(series[0]!.data.at(-2)).toEqual([60_000, 0.2])
    expect(series[0]!.data[4]).toEqual([54_000, 5])
    expect(series[0]!.markLine!.data).toEqual([{ xAxis: 55_000, name: '🌧' }])
    expect(series[1]!.type).toBe('scatter')
    expect(o.animation).toBe(false)
  })
})

describe('情绪日历（FR-DSH-03）', () => {
  it('月视图周一在前，前后补空；打字太少的天是“—”', () => {
    const weeks = monthGrid(
      '2026-10',
      [
        { date: '2026-10-01', dominant: 'low', has_diary: true },
        { date: '2026-10-02', dominant: null, has_diary: false },
      ],
      '2026-10-09',
    )
    expect(WEEK_HEAD).toEqual(['一', '二', '三', '四', '五', '六', '日'])
    // 2026-10-01 是周四
    expect(weeks[0]!.slice(0, 3)).toEqual([null, null, null])
    expect(weeks[0]![3]).toMatchObject({
      day: 1,
      icon: '🌧',
      hasDiary: true,
      label: '10月1日 周四，小雨，有日记',
    })
    expect(weeks[0]![4]).toMatchObject({ day: 2, icon: null, label: '10月2日 周五，—' })
    expect(weeks.flat().find((c) => c?.today)?.date).toBe('2026-10-09')
    expect(weeks.every((w) => w.length === 7)).toBe(true)
  })

  it('月份切换跨年；标题写成“2026年10月”', () => {
    expect(shiftMonth('2026-01', -1)).toBe('2025-12')
    expect(shiftMonth('2026-12', 1)).toBe('2027-01')
    expect(monthLabel('2026-10')).toBe('2026年10月')
  })
})

describe('周报（FR-DSH-04）', () => {
  const report: WeekReport = {
    week_start: '2026-10-05',
    days: Array.from({ length: 7 }, (_, i) => ({
      date: `2026-10-${String(5 + i).padStart(2, '0')}`,
      typing_min: i * 30,
      rests_due: 2,
      rests_done: 1,
      water: 1,
      focus_min: 0,
    })),
    states: [
      { state: 'fluent', windows: 30 },
      { state: 'tired', windows: 10 },
    ],
    heat: Array.from({ length: 7 }, (_, d) =>
      Array.from({ length: 24 }, (_, h) => (d === 2 && h === 23 ? 3 : 0)),
    ),
    rest_rate: 50,
    rests_done: 7,
    rests_due: 14,
    water: 7,
    schedules_added: 2,
    todos_done: 1,
    focus_min: 0,
    line: '这周晚上 11 点后更容易累，早点休息吧。',
  }

  it('状态分布：单色横条，带图标和名称，百分比', () => {
    const rows = distRows(report.states)
    expect(rows.map((r) => [r.name, r.pct])).toEqual([
      ['☀️ 晴', 75],
      ['🌙 夜', 25],
    ])
    expect(distLabel(rows)).toBe('状态分布：☀️ 晴 75%，🌙 夜 25%')
    const o = distOption(rows, C)
    const s = (o.series as { itemStyle: { color: string } }[])[0]!
    expect(s.itemStyle.color).toBe(C.series)
  })

  it('每日输入时长：单系列竖条，最宽 24 px、顶上 4 px 圆角', () => {
    const s = (
      typingOption(report.days, C).series as { barMaxWidth: number; itemStyle: { borderRadius: number[] } }[]
    )[0]!
    expect(s.barMaxWidth).toBe(24)
    expect(s.itemStyle.borderRadius).toEqual([4, 4, 0, 0])
  })

  it('热力图：单一色相由浅到深，格子间 2 px 底色缝', () => {
    const o = heatOption(report.heat, report.days, C)
    const vm = o.visualMap as { inRange: { color: string[] }; max: number }
    expect(vm.max).toBe(3)
    expect(vm.inRange.color[0]).toBe(C.empty)
    expect(vm.inRange.color.at(-1)).toBe(C.series)
    expect(mix('#ffffff', '#000000', 0.5)).toBe('#808080')
    expect(mix('#ffffff', 'CanvasText', 0.5)).toBe('CanvasText')
  })

  it('和上周比', () => {
    const pct = (n: number) => `${n}%`
    expect(versus(60, 50, pct)).toBe('比上周多 10%')
    expect(versus(40, 50, pct)).toBe('比上周少 10%')
    expect(versus(50, 50, pct)).toBe('和上周一样')
    expect(versus(50, null, pct)).toBe('上周没有数据')
    expect(weekRange(report)).toBe('10月5日 – 10月11日')
  })
})

describe('日程与待办（FR-SCH-09、FR-SCH-13）', () => {
  const item = (p: Partial<ScheduleItem>): ScheduleItem => ({
    id: 1,
    title: '组会',
    date: '2026-10-09',
    time: '15:00',
    end_time: null,
    all_day: false,
    location: null,
    is_deadline: false,
    remind_offsets: [600],
    status: 'added',
    source: 'manual',
    flags: [],
    created_ts: 0,
    ...p,
  })

  it('已添加的分成即将与已过期', () => {
    const now = new Date(2026, 9, 9, 16, 0)
    const { upcoming, expired } = splitAdded(
      [
        item({ id: 1, time: '15:00' }),
        item({ id: 2, time: '17:00' }),
        item({ id: 3, date: '2026-10-08' }),
        item({ id: 4, date: '2026-10-09', time: null, all_day: true }),
        item({ id: 5, time: '15:00', end_time: '17:00' }),
        item({ id: 6, date: null, time: null }),
      ],
      now,
    )
    expect(upcoming.map((s) => s.id)).toEqual([4, 5, 2, 6])
    expect(expired.map((s) => s.id)).toEqual([1, 3])
  })

  it('表单按后端规则校验，说清哪里不对', () => {
    const f = { ...emptySchedule('2026-10-09'), title: '组会', time: '15:00', end_time: '14:00' }
    expect(scheduleInput({ ...f, title: '  ' })).toEqual({
      ok: false,
      error: 'dashboard.schedule.title_required',
    })
    expect(scheduleInput({ ...f, title: '一二三四五六七八九十一二三' })).toMatchObject({
      ok: false,
      vars: { n: 12 },
    })
    expect(scheduleInput(f)).toEqual({ ok: false, error: 'dashboard.schedule.end_before_start' })
    expect(scheduleInput({ ...f, date: '', end_time: '' })).toEqual({
      ok: false,
      error: 'dashboard.schedule.time_needs_date',
    })
    const ok = scheduleInput({ ...f, end_time: '16:00', remind: '30', location: ' 实验楼 ' })
    expect(ok).toEqual({
      ok: true,
      input: {
        title: '组会',
        date: '2026-10-09',
        time: '15:00',
        end_time: '16:00',
        all_day: false,
        location: '实验楼',
        is_deadline: false,
        remind_offsets: [1800],
      },
    })
    // 全天：不带时刻，提醒按后端规则
    const allDay = scheduleInput({ ...f, all_day: true, remind: '30' })
    expect(allDay).toMatchObject({ ok: true, input: { time: null, end_time: null, remind_offsets: null } })
    expect(scheduleForm(item({ remind_offsets: [1800] })).remind).toBe('30')
    expect(scheduleForm(item({ remind_offsets: [86400, 7200] })).remind).toBe('default')
    expect(todoInput({ title: '打印简历', due_date: '' })).toEqual({
      ok: true,
      input: { title: '打印简历', due_date: null },
    })
    expect(todoInput({ title: '一二三四五六七八九十一二三四五六七', due_date: '' })).toMatchObject({
      ok: false,
      vars: { n: 16 },
    })
  })

  it('导航里的编辑目标', () => {
    expect(parseEdit('21')).toEqual({ kind: 'schedule', id: 21 })
    expect(parseEdit('todo:8')).toEqual({ kind: 'todo', id: 8 })
    expect(parseEdit('x')).toBeNull()
    expect(parseEdit(undefined)).toBeNull()
  })
})
