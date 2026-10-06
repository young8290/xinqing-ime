import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { Routine, RoutineNight, SelfReportItem } from '@/api'
import { FALLBACK_CHART_COLORS, chartColors } from '@/components/chartTheme'
import { t } from '@/i18n'
import { clock, routineLabel, routineOption, shortDate, stopTime, yRange } from './routine'

const night = (date: string, stop_min: number | null): RoutineNight => ({
  date,
  stop_min,
  stop_ts: stop_min === null ? null : 1,
  late: stop_min !== null && stop_min > 1440,
})
const ROUTINE: Routine = {
  nights: [
    night('2026-09-29', 1395),
    night('2026-09-30', null),
    night('2026-10-01', 1470),
    night('2026-10-02', 1410),
    night('2026-10-03', 1530),
    night('2026-10-04', 1380),
    night('2026-10-05', 1425),
  ],
  avg_stop_min: 1435,
  late_nights: 2,
  counted_nights: 6,
}

/** 测试里只看 option 的这几处 */
interface LineSeries {
  data: unknown[]
  connectNulls: boolean
  lineStyle: object
  symbolSize: number
  itemStyle: object
  markLine: { data: unknown }
}
interface Axis {
  data: string[]
  splitLine: { lineStyle: object }
  axisLabel: { color: string; formatter: (v: number) => string }
}
interface Tip {
  formatter: (p: { dataIndex: number }[]) => string
}

describe('作息洞察的纯函数', () => {
  it('停止打字时间：过了零点标“次日”', () => {
    expect(clock(1530)).toBe('01:30')
    expect(clock(1080)).toBe('18:00')
    expect(stopTime(1425)).toBe('23:45')
    expect(stopTime(1530)).toBe(`01:30${t('dashboard.routine.next_day')}`)
    expect(shortDate('2026-10-05')).toBe('10/5')
  })

  it('纵轴整点对齐，总能看到零点参考线，不超出 18:00～06:00', () => {
    expect(yRange(ROUTINE.nights)).toEqual({ min: 1200, max: 1560 })
    expect(yRange([night('2026-10-05', 1200)])).toEqual({ min: 1140, max: 1560 })
    expect(yRange([night('2026-10-05', 1790)])).toEqual({ min: 1200, max: 1800 })
    expect(yRange([night('2026-10-05', null)])).toEqual({ min: 1200, max: 1560 })
  })

  it('option 按图表规范：单系列 2 px 线、8 px 点带底色外圈、没记录的晚上断开、零点参考线、令牌配色', () => {
    const c = FALLBACK_CHART_COLORS
    const o = routineOption(ROUTINE, c)
    const s = (o.series as unknown as LineSeries[])[0]!
    expect(s.data).toEqual([1395, null, 1470, 1410, 1530, 1380, 1425])
    expect(s.connectNulls).toBe(false)
    expect(s.lineStyle).toMatchObject({ color: c.series, width: 2 })
    expect(s.symbolSize).toBe(8)
    expect(s.itemStyle).toMatchObject({ color: c.series, borderColor: c.surface, borderWidth: 2 })
    expect(s.markLine.data).toEqual([{ yAxis: 1440 }])
    expect(o.legend).toBeUndefined()
    const y = o.yAxis as unknown as Axis
    expect(y.splitLine.lineStyle).toMatchObject({ color: c.grid, width: 1, type: 'solid' })
    expect(y.axisLabel.formatter(1500)).toBe('01:00')
    // 轴上文字用文字令牌，不用系列色
    expect(y.axisLabel.color).toBe(c.axisText)
    expect((o.xAxis as unknown as Axis).data[0]).toBe('9/29')
    const tip = (o.tooltip as unknown as Tip).formatter
    expect(tip([{ dataIndex: 4 }])).toBe(
      `${t('dashboard.routine.night_of', { date: '10/3' })}<br/>${t('dashboard.routine.stopped_at', { time: stopTime(1530) })}`,
    )
    expect(tip([{ dataIndex: 1 }])).toContain(t('dashboard.routine.no_record'))
  })

  it('读屏概括带平均时间与晚于零点的晚数', () => {
    expect(routineLabel(ROUTINE)).toBe(t('dashboard.routine.chart_label', { n: 7, avg: '23:55', late: 2 }))
    expect(routineLabel({ ...ROUTINE, avg_stop_min: null })).toContain(t('dashboard.routine.no_avg'))
  })

  it('图表颜色取自设计令牌，读不到时用浅色值', () => {
    const el = document.createElement('div')
    el.style.setProperty('--xq-chart-1', '#c98128')
    el.style.setProperty('--xq-surface', '#252321')
    document.body.append(el)
    expect(chartColors(el)).toMatchObject({
      series: '#c98128',
      surface: '#252321',
      grid: FALLBACK_CHART_COLORS.grid,
    })
    el.remove()
  })

  it('文案不说睡眠（FR-REV-03 文案约束）', () => {
    const texts = [
      t('dashboard.routine.title'),
      t('dashboard.routine.avg'),
      t('dashboard.routine.no_record'),
      t('dashboard.routine.chart_label'),
      t('routine.stop_typing_note'),
    ].join('')
    expect(texts).not.toMatch(/睡眠|失眠/)
  })
})

const mocks = vi.hoisted(() => ({
  getRoutine: vi.fn(),
  reports: [] as SelfReportItem[],
  /** 最近一次交给图表组件的参数 */
  chart: null as null | { option: (c: unknown) => unknown; label: string },
}))
const ok = (data: unknown) => Promise.resolve({ status: 'ok', data })

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    getRoutine: mocks.getRoutine,
    selfReportList: async () => ok(mocks.reports),
    getStatus: async () => ok(null),
  },
  events: { statusChanged: { listen: async () => () => {} } },
}))
// jsdom 没有布局，ECharts 量不出尺寸；看板测的是交给图表的数据，图表组件换成桩
vi.mock('@/components/EChart.vue', () => ({
  default: defineComponent({
    props: { option: { type: Function, required: true }, label: { type: String, required: true } },
    setup(props) {
      return () => {
        mocks.chart = props as never
        return h('div', { class: 'chart-stub', 'aria-label': props.label })
      }
    },
  }),
}))

const { default: App } = await import('./App.vue')

enableAutoUnmount(afterEach)

describe('情绪看板', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    location.hash = ''
    mocks.getRoutine
      .mockReset()
      .mockImplementation((days: number) =>
        ok(
          days === 7
            ? ROUTINE
            : { ...ROUTINE, nights: [...ROUTINE.nights, ...ROUTINE.nights], counted_nights: 12 },
        ),
      )
    mocks.reports = []
    mocks.chart = null
  })

  it('左侧导航六页（FR-DSH-01），默认“今日”', async () => {
    const w = mount(App)
    await flushPromises()
    expect(w.findAll('nav button').map((b) => b.text())).toEqual([
      '今日',
      '情绪日历',
      '周报',
      '日程与待办',
      '信箱',
      '对话与日记',
    ])
    expect(w.find('h1').text()).toBe('今日')
    await w.findAll('nav button')[1]!.trigger('click')
    expect(w.find('h1').text()).toBe('情绪日历')
    expect(w.text()).toContain(t('dashboard.coming'))
    // 页名不落到根元素上变成悬停提示
    expect(w.find('main section').attributes('title')).toBeUndefined()
  })

  it('今日：今天的自评按时间倒序，只显示选项，不显示备注（看板不显示原文）', async () => {
    mocks.reports = [
      {
        id: 1,
        ts: new Date(2026, 9, 5, 9, 5).getTime(),
        weather: 'sunny',
        note: '早上心情不错',
        auto_state: null,
      },
      {
        id: 2,
        ts: new Date(2026, 9, 5, 15, 40).getTime(),
        weather: 'night',
        note: '有点困',
        auto_state: 'tired',
      },
    ]
    const w = mount(App)
    await flushPromises()
    const items = w.findAll('.reports li')
    expect(items.map((li) => li.text())).toEqual([
      `15:40 ${t('self_report.options.night')}`,
      `09:05 ${t('self_report.options.sunny')}`,
    ])
    expect(w.text()).not.toContain('早上心情不错')
    expect(w.text()).not.toContain('有点困')
  })

  it('今日：没有自评时说明怎么记', async () => {
    const w = mount(App)
    await flushPromises()
    expect(w.text()).toContain(t('dashboard.self_reports_empty'))
  })

  it('周报：作息洞察的数字、图表与“按天查看”表格；切到 30 天重新取', async () => {
    location.hash = '#weekly'
    const w = mount(App)
    await flushPromises()
    expect(w.find('h1').text()).toBe('周报')
    expect(mocks.getRoutine).toHaveBeenCalledWith(7)
    const stats = w.findAll('.stats dd').map((d) => d.text())
    expect(stats).toEqual(['23:55', '2 晚', '6 / 7 晚'])
    expect(w.find('.chart-stub').attributes('aria-label')).toBe(routineLabel(ROUTINE))
    // 交给图表的就是这 7 晚的数据（option 里的格式化函数每次新建，只比数据）
    const given = mocks.chart!.option(FALLBACK_CHART_COLORS) as ReturnType<typeof routineOption>
    expect((given.series as { data: unknown }[])[0]!.data).toEqual(ROUTINE.nights.map((n) => n.stop_min))
    expect(w.text()).toContain(t('routine.stop_typing_note'))

    const rows = w.findAll('.table tbody tr')
    expect(rows).toHaveLength(7)
    expect(rows[0]!.text()).toBe('10/5' + '23:45')
    expect(rows[2]!.text()).toContain(stopTime(1530))
    expect(rows[2]!.find('.tag').text()).toBe(t('dashboard.routine.late'))
    expect(rows[5]!.text()).toContain(t('dashboard.routine.no_record'))

    const seg = w.findAll('.seg button')
    expect(seg[0]!.attributes('aria-pressed')).toBe('true')
    await seg[1]!.trigger('click')
    await flushPromises()
    expect(mocks.getRoutine).toHaveBeenLastCalledWith(30)
    expect(w.findAll('.seg button')[1]!.attributes('aria-pressed')).toBe('true')
    expect(w.findAll('.stats dd')[2]!.text()).toBe('12 / 14 晚')
  })

  it('周报：一晚记录都没有时不画图，说明原因', async () => {
    mocks.getRoutine.mockImplementation(() =>
      ok({ nights: [night('2026-10-05', null)], avg_stop_min: null, late_nights: 0, counted_nights: 0 }),
    )
    location.hash = '#weekly'
    const w = mount(App)
    await flushPromises()
    expect(w.find('.chart-stub').exists()).toBe(false)
    expect(w.text()).toContain(t('dashboard.routine.none'))
    expect(w.findAll('.stats dd')[0]!.text()).toBe(t('dashboard.routine.no_avg'))
  })
})
