import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type {
  ChatSessionItem,
  ComfortItem,
  DayMood,
  DayStats,
  DiaryItem,
  LetterItem,
  Routine,
  RoutineNight,
  ScheduleItem,
  SelfReportItem,
  TodoItem,
  WeekReport,
} from '@/api'
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
  /** 交给图表组件的参数，按读屏概括（label）记 */
  charts: {} as Record<string, { option: (c: unknown) => unknown; label: string }>,
  stats: null as unknown,
  comforts: [] as unknown[],
  moods: [] as unknown[],
  week: null as unknown,
  diaries: [] as unknown[],
  sessions: [] as unknown[],
  letters: [] as unknown[],
  schedules: {} as Record<string, unknown[]>,
  todos: {} as Record<string, unknown[]>,
  call: vi.fn(),
  emit: vi.fn(),
  listeners: {} as Record<string, (e: { payload: unknown }) => void>,
}))
const ok = (data: unknown) => Promise.resolve({ status: 'ok', data })
/** 记一笔调用并返回成功 */
const rec =
  (name: string, data: unknown = null) =>
  (...args: unknown[]) => {
    mocks.call(name, ...args)
    return ok(data)
  }

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    getRoutine: mocks.getRoutine,
    selfReportList: async () => ok(mocks.reports),
    getStatus: async () => ok(null),
    dayStats: (date: string) => {
      mocks.call('dayStats', date)
      return ok({ ...(mocks.stats as object), date })
    },
    comfortList: async () => ok(mocks.comforts),
    comfortFeedback: rec('comfortFeedback'),
    stateExplain: (id: number) => {
      mocks.call('stateExplain', id)
      return ok({
        state: 'tired',
        prob_pct: null,
        signals: [{ kind: 'session', value: 50 }],
        source: 'rule',
        cold_start: false,
      })
    },
    monthMoods: (m: string) => {
      mocks.call('monthMoods', m)
      return ok(mocks.moods)
    },
    weekStats: (w: string) => {
      mocks.call('weekStats', w)
      return ok({ ...(mocks.week as object), week_start: w })
    },
    diaryList: async () => ok(mocks.diaries),
    diaryGenerate: rec('diaryGenerate', {
      draft_id: 9,
      text: '今天有点累，但完成了作业。',
      ai_generated: true,
    }),
    diarySave: rec('diarySave', { id: 1, source: 'ai_draft', safety: false }),
    diaryDelete: rec('diaryDelete'),
    chatListSessions: async () => ok(mocks.sessions),
    chatDelete: rec('chatDelete'),
    lettersList: async () => ok(mocks.letters),
    letterRead: rec('letterRead'),
    letterDelete: rec('letterDelete'),
    scheduleList: async (st: string) => ok(mocks.schedules[st] ?? []),
    todoList: async (st: string) => ok(mocks.todos[st] ?? []),
    scheduleConfirm: rec('scheduleConfirm'),
    scheduleIgnore: rec('scheduleIgnore'),
    scheduleCreate: rec('scheduleCreate', {
      id: 5,
      conflicts: [{ id: 2, title: '班会', start: '15:00', end: '16:00' }],
    }),
    scheduleUpdate: rec('scheduleUpdate', { id: 5, conflicts: [] }),
    scheduleDelete: rec('scheduleDelete'),
    scheduleExportIcs: rec('scheduleExportIcs', '/tmp/x.ics'),
    todoComplete: rec('todoComplete'),
    todoCreate: rec('todoCreate', 3),
    openWindow: rec('openWindow'),
  },
  events: {
    statusChanged: { listen: async () => () => {} },
    typingPulse: { listen: async () => () => {} },
  },
}))
vi.mock('@tauri-apps/api/event', () => ({
  emit: mocks.emit,
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    mocks.listeners[name] = cb
    return () => {}
  },
}))
// jsdom 没有布局，ECharts 量不出尺寸；看板测的是交给图表的数据，图表组件换成桩
vi.mock('@/components/EChart.vue', () => ({
  default: defineComponent({
    props: { option: { type: Function, required: true }, label: { type: String, required: true } },
    setup(props) {
      return () => {
        mocks.charts[props.label] = props as never
        return h('div', { class: 'chart-stub', 'aria-label': props.label })
      }
    },
  }),
}))

const { default: App } = await import('./App.vue')

enableAutoUnmount(afterEach)

const TODAY = new Date()
const pad = (n: number) => String(n).padStart(2, '0')
const today = `${TODAY.getFullYear()}-${pad(TODAY.getMonth() + 1)}-${pad(TODAY.getDate())}`
const at = (h: number, m = 0) =>
  new Date(TODAY.getFullYear(), TODAY.getMonth(), TODAY.getDate(), h, m).getTime()

const STATS: Omit<DayStats, 'date'> = {
  typing_min: 102,
  rests_due: 4,
  rests_done: 3,
  water: 2,
  comforts: 2,
  dominant: 'tired',
  timeline: [{ start_ts: at(9), end_ts: at(12), state: 'tired', mood_id: 42 }],
}

function week(): Omit<WeekReport, 'week_start'> {
  return {
    days: Array.from({ length: 7 }, (_, i) => ({
      date: `2026-10-${pad(5 + i)}`,
      typing_min: 60 * i,
      rests_due: 2,
      rests_done: 1,
      water: 1,
      focus_min: 0,
    })),
    states: [
      { state: 'fluent', windows: 3 },
      { state: 'tired', windows: 1 },
    ],
    heat: Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => 0)),
    rest_rate: 50,
    rests_done: 7,
    rests_due: 14,
    water: 7,
    schedules_added: 2,
    todos_done: 1,
    focus_min: 0,
    line: '这周休息完成了 50%。',
  }
}

const sch = (p: Partial<ScheduleItem>): ScheduleItem => ({
  id: 1,
  title: '组会',
  date: '2099-10-09',
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

async function mountAt(hash = '') {
  location.hash = hash
  const w = mount(App)
  await flushPromises()
  return w
}

beforeEach(() => {
  setActivePinia(createPinia())
  location.hash = ''
  vi.spyOn(console, 'warn').mockImplementation(() => {})
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
  mocks.charts = {}
  mocks.stats = STATS
  mocks.comforts = []
  mocks.moods = []
  mocks.week = week()
  mocks.diaries = []
  mocks.sessions = []
  mocks.letters = []
  mocks.schedules = {}
  mocks.todos = {}
  mocks.call.mockReset()
  mocks.emit.mockReset().mockResolvedValue(undefined)
  mocks.listeners = {}
  localStorage.clear()
})

describe('情绪看板', () => {
  it('左侧导航六页（FR-DSH-01），默认“今日”', async () => {
    const w = await mountAt()
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
    await flushPromises()
    expect(w.find('h1').text()).toBe('情绪日历')
    // 页名不落到根元素上变成悬停提示
    expect(w.find('main section').attributes('title')).toBeUndefined()
  })

  it('别的窗口要看板跳页：打开时取 localStorage，之后听事件', async () => {
    localStorage.setItem('xq.dashboard.nav', JSON.stringify({ page: 'mailbox' }))
    const w = await mountAt()
    expect(w.find('h1').text()).toBe('信箱')
    expect(localStorage.getItem('xq.dashboard.nav')).toBeNull()
    mocks.listeners['xq:dashboard-nav']!({ payload: { page: 'schedule' } })
    await flushPromises()
    expect(w.find('h1').text()).toBe('日程与待办')
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
    const w = await mountAt()
    const items = w.findAll('.reports li')
    expect(items.map((li) => li.text())).toEqual([
      `15:40 ${t('self_report.options.night')}`,
      `09:05 ${t('self_report.options.sunny')}`,
    ])
    expect(w.text()).not.toContain('早上心情不错')
    expect(w.text()).not.toContain('有点困')
  })

  it('今日：没有自评时说明怎么记', async () => {
    const w = await mountAt()
    expect(w.text()).toContain(t('dashboard.self_reports_empty'))
  })
})

describe('今日（FR-DSH-02）', () => {
  it('四张概要卡片、时间线点开看解释', async () => {
    const w = await mountAt()
    expect(mocks.call).toHaveBeenCalledWith('dayStats', today)
    expect(w.findAll('.tile-value').map((v) => v.text())).toEqual(['1 小时 42 分', '🌙 夜', '2 次', '75%'])
    const seg = w.find('button.seg')
    expect(seg.attributes('aria-label')).toBe('09:00–12:00 夜')
    await seg.trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('stateExplain', 42)
    expect(w.find('.explain h3').text()).toBe('09:00–12:00 的判断')
  })

  it('今日一句：AI 标识、反馈状态，没反馈的可以点', async () => {
    mocks.comforts = [
      { id: 1, ts: at(9), text: '慢慢来。', ai_generated: false, trigger: 'auto', feedback: 'useful' },
      { id: 2, ts: at(10), text: '辛苦了。', ai_generated: true, trigger: 'self_report', feedback: null },
    ] satisfies ComfortItem[]
    const w = await mountAt()
    const rows = w.findAll('.comfort')
    expect(rows[0]!.text()).toContain('👍 有用')
    expect(rows[1]!.find('.d-tag').text()).toBe('AI 生成')
    await rows[1]!.find('button[data-verdict=unfit]').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('comfortFeedback', 2, 'unfit')
    expect(w.findAll('.comfort')[1]!.text()).toContain('👎 不合适')
  })

  it('写今天的日记：先请晴晴起草，草稿标“AI 生成”，改过变“AI 辅助生成”，保存', async () => {
    const w = await mountAt()
    await w.find('.d-head button.primary').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('diaryGenerate', null)
    const area = w.find('textarea')
    expect((area.element as HTMLTextAreaElement).value).toBe('今天有点累，但完成了作业。')
    expect(w.find('.diary-editor .tag').text()).toBe('AI 生成')
    await area.setValue('今天有点累。')
    expect(w.find('.diary-editor .tag').text()).toBe('AI 辅助生成')
    await w.find('.diary-editor').trigger('submit')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('diarySave', null, 9, '今天有点累。')
    expect(w.find('.diary-editor').exists()).toBe(false)
    expect(w.text()).toContain(t('diary.saved'))
  })
})

describe('情绪日历（FR-DSH-03）', () => {
  it('每天主导天气、日记标记；点某天看时间线和日记', async () => {
    const month = today.slice(0, 7)
    mocks.moods = [
      { date: `${month}-01`, dominant: 'low', has_diary: true },
      { date: `${month}-02`, dominant: null, has_diary: false },
    ] satisfies DayMood[]
    mocks.diaries = [
      { id: 1, date: `${month}-01`, content: '那天写的', source: 'ai_edited', created_ts: 0, updated_ts: 0 },
      { id: 2, date: `${month}-03`, content: '别的天', source: 'manual', created_ts: 0, updated_ts: 0 },
    ] satisfies DiaryItem[]
    const w = await mountAt('#calendar')
    expect(mocks.call).toHaveBeenCalledWith('monthMoods', month)
    const first = w.findAll('button.cell')[0]!
    expect(first.text()).toContain('🌧')
    expect(first.text()).toContain('📓')
    expect(w.findAll('button.cell')[1]!.text()).toContain('—')
    await first.trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('dayStats', `${month}-01`)
    expect(w.find('.detail').text()).toContain('那天写的')
    expect(w.find('.detail').text()).not.toContain('别的天')
    expect(w.find('.detail .d-tag').text()).toBe('AI 辅助生成')
    await w.find('nav ~ main .d-head button[aria-label="下个月"]').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenLastCalledWith('monthMoods', expect.stringMatching(/^\d{4}-\d{2}$/))
  })
})

describe('周报（FR-DSH-04）', () => {
  it('一句话、和上周比、状态分布与每日输入时长的图、按天查看', async () => {
    const w = await mountAt('#weekly')
    expect(w.find('.line').text()).toBe('这周休息完成了 50%。')
    expect(w.findAll('.tile-value').map((v) => v.text())).toEqual(['50%', '7 次', '2 个', '1 个'])
    expect(w.findAll('.tile-detail').map((v) => v.text())).toEqual(['和上周一样', '和上周一样'])
    expect(Object.keys(mocks.charts)).toContain('状态分布：☀️ 晴 75%，🌙 夜 25%')
    expect(w.findAll('.d-table tbody tr')).toHaveLength(7)
    expect(w.text()).toContain(t('dashboard.weekly.heat_none'))
  })

  it('作息洞察：数字、图表与“按天查看”表格；切到 30 天重新取', async () => {
    const w = await mountAt('#weekly')
    expect(mocks.getRoutine).toHaveBeenCalledWith(7)
    const stats = w.findAll('.stats dd').map((d) => d.text())
    expect(stats).toEqual(['23:55', '2 晚', '6 / 7 晚'])
    const chart = mocks.charts[routineLabel(ROUTINE)]!
    // 交给图表的就是这 7 晚的数据（option 里的格式化函数每次新建，只比数据）
    const given = chart.option(FALLBACK_CHART_COLORS) as ReturnType<typeof routineOption>
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

  it('作息洞察：一晚记录都没有时不画图，说明原因', async () => {
    mocks.getRoutine.mockImplementation(() =>
      ok({ nights: [night('2026-10-05', null)], avg_stop_min: null, late_nights: 0, counted_nights: 0 }),
    )
    const w = await mountAt('#weekly')
    expect(w.find('.routine .chart-stub').exists()).toBe(false)
    expect(w.text()).toContain(t('dashboard.routine.none'))
    expect(w.findAll('.stats dd')[0]!.text()).toBe(t('dashboard.routine.no_avg'))
  })
})

describe('日程与待办（FR-SCH-09、FR-SCH-13）', () => {
  it('待确认：带 AI 识别与提示，可以添加 / 忽略', async () => {
    mocks.schedules = { pending: [sch({ id: 7, status: 'pending', source: 'ai', flags: ['local'] })] }
    const w = await mountAt('#schedule')
    const item = w.findAll('.d-list')[0]!.find('.item')
    expect(item.text()).toContain('AI 识别')
    expect(item.text()).toContain('请确认信息')
    await item.find('button.primary').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('scheduleConfirm', 7)
  })

  it('新建日程：先校验，保存后提示时间重叠（只提示不阻止）', async () => {
    const w = await mountAt('#schedule')
    await w.findAll('.d-head .d-row button')[0]!.trigger('click')
    const form = w.find('form.editor')
    await form.trigger('submit')
    expect(form.find('[role=alert]').text()).toBe(t('dashboard.schedule.title_required'))
    await form.find('input').setValue('组会')
    await form.find('input[type=time]').setValue('15:30')
    await form.trigger('submit')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith(
      'scheduleCreate',
      expect.objectContaining({ title: '组会', time: '15:30', date: today }),
    )
    expect(w.find('[role=status]').text()).toBe('已保存。⚠ 与「班会」时间重叠（15:00–16:00）')
  })

  it('卡片层“修改”跳过来：直接打开那一条的编辑框', async () => {
    mocks.schedules = { added: [sch({ id: 21 })] }
    localStorage.setItem('xq.dashboard.nav', JSON.stringify({ page: 'schedule', edit: '21' }))
    const w = await mountAt()
    expect(w.find('h1').text()).toBe('日程与待办')
    expect((w.find('form.editor input').element as HTMLInputElement).value).toBe('组会')
  })

  it('已添加：删除要确认，可以加入我的日历；待办可以完成', async () => {
    mocks.schedules = {
      added: [sch({ id: 1 }), sch({ id: 2, title: '晚课', time: '19:00' })],
      ignored: [sch({ id: 3, title: null })],
    }
    mocks.todos = {
      open: [
        {
          id: 8,
          title: '打印简历',
          due_date: null,
          status: 'open',
          source: 'manual',
          created_ts: 0,
          done_ts: null,
        } satisfies TodoItem,
      ],
    }
    const w = await mountAt('#schedule')
    expect(w.text()).toContain('另外忽略过 1 个')
    await w.find('button[aria-label="完成「打印简历」"]').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('todoComplete', 8)
    const exportAll = w.findAll('button').find((b) => b.text() === '全部加入我的日历')!
    await exportAll.trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('scheduleExportIcs', [1, 2])
    const del = () => w.findAll('button').find((b) => b.text() === '删除')!
    await del().trigger('click')
    expect(mocks.call).not.toHaveBeenCalledWith('scheduleDelete', 1)
    await w
      .findAll('button')
      .find((b) => b.text() === '确定删除')!
      .trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('scheduleDelete', 1)
  })
})

describe('信箱（FR-DSH-06）', () => {
  const letter = (p: Partial<LetterItem>): LetterItem => ({
    id: 1,
    week_start: '2026-10-05',
    content: '这周你打字的时间比上周少了一些。',
    ai_generated: true,
    read: false,
    created_ts: 0,
    ...p,
  })

  it('未读有圆点和文字；打开标已读，信末 AI 生成；删除要确认', async () => {
    mocks.letters = [
      letter({ id: 2 }),
      letter({ id: 1, read: true, ai_generated: false, week_start: '2026-09-28' }),
    ]
    const w = await mountAt('#mailbox')
    const rows = w.findAll('.letters button.row')
    expect(rows[0]!.text()).toContain('未读')
    expect(rows[1]!.text()).not.toContain('未读')
    await rows[0]!.trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('letterRead', 2)
    expect(w.find('.paper .body').text()).toBe('这周你打字的时间比上周少了一些。')
    expect(w.find('.paper .sign').text()).toBe('AI 生成')
    await w
      .findAll('.paper button')
      .find((b) => b.text() === t('dashboard.mailbox.delete'))!
      .trigger('click')
    await w.find('.paper button.danger').trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('letterDelete', 2)
  })

  it('从卡片层“打开看看”跳过来：直接打开那一封', async () => {
    mocks.letters = [letter({ id: 4 })]
    localStorage.setItem('xq.dashboard.nav', JSON.stringify({ page: 'mailbox', letter: 4 }))
    const w = await mountAt()
    await flushPromises()
    expect(w.find('.paper').exists()).toBe(true)
  })
})

describe('对话与日记（FR-DSH-05、FR-DIA-03、FR-CHT-08）', () => {
  it('日记的来源标识；对话可以在对话窗口打开', async () => {
    mocks.diaries = [
      { id: 1, date: '2026-10-05', content: '草稿', source: 'ai_draft', created_ts: 0, updated_ts: 0 },
      { id: 2, date: '2026-10-04', content: '手写', source: 'manual', created_ts: 0, updated_ts: 0 },
    ] satisfies DiaryItem[]
    mocks.sessions = [
      { id: 6, title: '今天好累', created_ts: at(9), last_ts: at(10), safe_mode: 'off', mode: 'normal' },
    ] satisfies ChatSessionItem[]
    const w = await mountAt('#chat_diary')
    const tags = w.findAll('article .d-tag').map((x) => x.text())
    expect(tags).toEqual(['AI 生成'])
    await w
      .findAll('button')
      .find((b) => b.text() === '打开这段对话')!
      .trigger('click')
    await flushPromises()
    expect(mocks.call).toHaveBeenCalledWith('openWindow', 'chat')
    expect(mocks.emit).toHaveBeenCalledWith('xq:chat-open', 6)
  })
})
