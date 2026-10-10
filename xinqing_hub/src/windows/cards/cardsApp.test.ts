import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type {
  ComfortNew,
  EveningSummary,
  LetterNew,
  ReminderDue,
  ReminderMissed,
  RestDue,
  ScheduleItem,
  ScheduleReady,
  TodoReady,
} from '@/api'
import { t } from '@/i18n'

type Cb = (e: { payload: unknown }) => void
const mocks = vi.hoisted(() => ({
  emit: {} as Record<string, Cb>,
  missed: null as unknown,
  scheduleConfirm: vi.fn(),
  scheduleIgnore: vi.fn(),
  todoConfirm: vi.fn(),
  todoIgnore: vi.fn(),
  reminderAction: vi.fn(),
  restAction: vi.fn(),
  openWindow: vi.fn(),
  busEmit: vi.fn(),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })
const listener = (name: string) => ({
  listen: async (cb: Cb) => {
    mocks.emit[name] = cb
    return () => {}
  },
})

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    settingsGet: () => ok(true),
    reminderMissedTake: () => {
      const m = mocks.missed
      mocks.missed = null
      return Promise.resolve(m)
    },
    scheduleConfirm: mocks.scheduleConfirm,
    scheduleIgnore: mocks.scheduleIgnore,
    todoConfirm: mocks.todoConfirm,
    todoIgnore: mocks.todoIgnore,
    reminderAction: mocks.reminderAction,
    restAction: mocks.restAction,
    openWindow: mocks.openWindow,
  },
  events: {
    settingsChanged: { listen: async () => () => {} },
    scheduleDetected: listener('scheduleDetected'),
    scheduleReady: listener('scheduleReady'),
    todoDetected: listener('todoDetected'),
    todoReady: listener('todoReady'),
    reminderDue: listener('reminderDue'),
    reminderMissed: listener('reminderMissed'),
    restDue: listener('restDue'),
    comfortNew: listener('comfortNew'),
    reviewEvening: listener('reviewEvening'),
    letterNew: listener('letterNew'),
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ emit: mocks.busEmit, listen: async () => () => {} }))
// 定位要的窗口接口不给：useCardsWindow 失败只打警告，卡片照常渲染
vi.mock('@tauri-apps/api/window', () => ({
  LogicalSize: class {},
  PhysicalPosition: class {},
  Window: { getByLabel: async () => null },
  getCurrentWindow: () => ({ isVisible: async () => false, hide: async () => {} }),
  monitorFromPoint: async () => null,
}))

const { default: App } = await import('./App.vue')

function sch(p: Partial<ScheduleItem> = {}): ScheduleItem {
  return {
    id: 21,
    title: '组会',
    date: '2026-10-09',
    time: '15:00',
    end_time: null,
    all_day: false,
    location: '实验楼',
    is_deadline: false,
    remind_offsets: [600],
    status: 'pending',
    source: 'ai',
    flags: [],
    created_ts: 0,
    ...p,
  }
}

async function mountCards() {
  const w = mount(App)
  await flushPromises()
  return w
}

const fire = async (name: string, payload: unknown) => {
  mocks.emit[name]!({ payload })
  await flushPromises()
}

const cards = (w: Awaited<ReturnType<typeof mountCards>>) => w.findAll('[role=alertdialog]')
const buttons = (el: { findAll: (s: string) => { text: () => string }[] }) =>
  el.findAll('.actions button').map((b) => b.text())

beforeEach(() => {
  setActivePinia(createPinia())
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  mocks.emit = {}
  mocks.missed = null
  for (const f of [
    mocks.scheduleConfirm,
    mocks.scheduleIgnore,
    mocks.todoConfirm,
    mocks.todoIgnore,
    mocks.reminderAction,
    mocks.restAction,
    mocks.openWindow,
  ])
    f.mockReset().mockReturnValue(ok())
  mocks.busEmit.mockReset().mockResolvedValue(undefined)
})

describe('日程与待办卡片（FR-SCH-05、FR-SCH-11、FR-SCH-13）', () => {
  it('先“识别中…”，抽取完填字段、带 AI 识别、提示与时间重叠', async () => {
    const w = await mountCards()
    await fire('scheduleDetected', { card_id: 3 })
    expect(cards(w)[0]!.text()).toContain('📅 识别中…')
    const ready: ScheduleReady = {
      card_id: 3,
      schedule: sch({ flags: ['local'] }),
      conflicts: [{ id: 9, title: '班会', start: '15:00', end: '16:00' }],
    }
    await fire('scheduleReady', ready)
    const card = cards(w)[0]!
    expect(card.find('.title').text()).toBe('📅 组会')
    expect(card.find('.tag').text()).toBe('AI 识别')
    expect(card.text()).toContain('10月9日 周五 15:00')
    expect(card.text()).toContain('📍 实验楼')
    expect(card.text()).toContain('请确认信息')
    expect(card.text()).toContain('⚠ 与「班会」时间重叠（15:00–16:00）')
    expect(buttons(card)).toEqual(['添加', '修改', '忽略'])
  })

  it('添加：调命令、显示“已添加 ✓”后收起；告诉小组件卡片数', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      const w = await mountCards()
      await fire('scheduleReady', { card_id: 3, schedule: sch(), conflicts: [] })
      expect(mocks.busEmit).toHaveBeenLastCalledWith('xq:cards', { count: 1 })
      await cards(w)[0]!.find('button.primary').trigger('click')
      await flushPromises()
      expect(mocks.scheduleConfirm).toHaveBeenCalledWith(21)
      expect(w.find('.done').text()).toBe('已添加 ✓')
      await vi.advanceTimersByTimeAsync(1500)
      expect(cards(w)).toHaveLength(0)
      expect(mocks.busEmit).toHaveBeenLastCalledWith('xq:cards', { count: 0 })
    } finally {
      vi.useRealTimers()
    }
  })

  it('修改打开看板日程页的编辑框；忽略调命令；24 小时内提示过的直接收起', async () => {
    const w = await mountCards()
    await fire('scheduleReady', { card_id: 3, schedule: sch(), conflicts: [] })
    await cards(w)[0]!.findAll('.actions button')[1]!.trigger('click')
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('dashboard')
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:dashboard-nav', { page: 'schedule', edit: '21' })
    expect(cards(w)).toHaveLength(0)
    await fire('scheduleReady', { card_id: 4, schedule: sch({ id: 22 }), conflicts: [] })
    await cards(w)[0]!.findAll('.actions button')[2]!.trigger('click')
    await flushPromises()
    expect(mocks.scheduleIgnore).toHaveBeenCalledWith(22)
    await fire('scheduleDetected', { card_id: 5 })
    await fire('scheduleReady', { card_id: 5, schedule: null, conflicts: [] })
    expect(cards(w)).toHaveLength(0)
  })

  it('60 秒没操作就收起（留在待确认列表）', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      const w = await mountCards()
      await fire('scheduleReady', { card_id: 3, schedule: sch(), conflicts: [] })
      await vi.advanceTimersByTimeAsync(59_000)
      expect(cards(w)).toHaveLength(1)
      await vi.advanceTimersByTimeAsync(1_000)
      expect(cards(w)).toHaveLength(0)
    } finally {
      vi.useRealTimers()
    }
  })

  it('待办卡片：加入待办 / 修改 / 忽略，带截止日期', async () => {
    const w = await mountCards()
    const ready: TodoReady = {
      card_id: 1,
      todo: {
        id: 8,
        title: '打印简历',
        due_date: '2026-10-04',
        status: 'pending',
        source: 'ai',
        created_ts: 0,
        done_ts: null,
      },
    }
    await fire('todoReady', ready)
    const card = cards(w)[0]!
    expect(card.find('.title').text()).toBe('✅ 打印简历')
    expect(card.text()).toContain('截止 10月4日')
    expect(buttons(card)).toEqual(['加入待办', '修改', '忽略'])
    await card.find('button.primary').trigger('click')
    await flushPromises()
    expect(mocks.todoConfirm).toHaveBeenCalledWith(8)
    expect(w.find('.done').text()).toBe('已加入待办 ✓')
  })
})

describe('提醒卡片（FR-SCH-07、FR-SCH-14）', () => {
  const due = (p: Partial<ReminderDue> = {}): ReminderDue => ({
    id: 5,
    kind: 'schedule',
    ref_id: 21,
    title: '组会',
    body: '15:00 开始，实验楼',
    ...p,
  })

  it('到点：知道了 / 5 分钟后 / 10 分钟后；Esc 等同 5 分钟后', async () => {
    const w = await mountCards()
    await fire('reminderDue', due())
    const card = cards(w)[0]!
    expect(card.text()).toContain('15:00 开始，实验楼')
    expect(buttons(card)).toEqual(['知道了', '5 分钟后', '10 分钟后'])
    await card.findAll('.actions button')[2]!.trigger('click')
    await flushPromises()
    expect(mocks.reminderAction).toHaveBeenLastCalledWith(5, 'snooze_10')
    await fire('reminderDue', due({ id: 6 }))
    await cards(w)[0]!.trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(mocks.reminderAction).toHaveBeenLastCalledWith(6, 'snooze_5')
  })

  it('每日汇总只有“知道了”', async () => {
    const w = await mountCards()
    await fire('reminderDue', due({ kind: 'todo_digest', ref_id: 0 }))
    expect(buttons(cards(w)[0]!)).toEqual(['知道了'])
  })

  it('错过的提醒：打开时取一次，列出每一条，只显示一次（ADR 0036）', async () => {
    const missed: ReminderMissed = {
      title: '错过的提醒',
      body: '有 2 个提醒在你离开时到期了',
      items: [due({ id: 1, title: '组会' }), due({ id: 2, title: '交报告', body: '18:00 截止' })],
    }
    mocks.missed = missed
    const w = await mountCards()
    const card = cards(w)[0]!
    expect(card.findAll('li').map((li) => li.text())).toEqual([
      '组会 · 15:00 开始，实验楼',
      '交报告 · 18:00 截止',
    ])
    expect(buttons(card)).toEqual(['知道了', '看看日程'])
    // 事件也来了：后端已经给过了，不会再来一张
    await fire('reminderMissed', missed)
    expect(cards(w)).toHaveLength(1)
  })
})

describe('休息提醒卡片（FR-RST-02～06）', () => {
  it('喝水：卡片给三个按钮，“已完成”叫“喝了”；点了就收起并交给后端，不会打开对话', async () => {
    const w = await mountCards()
    await fire('restDue', { kind: 'water', tired: false } satisfies RestDue)
    const card = cards(w)[0]!
    expect(card.text()).toContain(t('rest.water'))
    expect(buttons(card)).toEqual([t('rest.btn_drank'), t('rest.btn_later'), t('rest.btn_today_off')])
    await card.find('button.primary').trigger('click')
    await flushPromises()
    expect(mocks.restAction).toHaveBeenCalledWith('water', 'done')
    expect(cards(w)).toHaveLength(0)
    expect(mocks.openWindow).not.toHaveBeenCalled()
  })

  it('“今天不再提醒”与 Esc（等同 5 分钟后）；新的休息提醒替换旧的', async () => {
    const w = await mountCards()
    await fire('restDue', { kind: 'move', tired: false })
    await cards(w)[0]!.findAll('.actions button')[2]!.trigger('click')
    await flushPromises()
    expect(mocks.restAction).toHaveBeenLastCalledWith('move', 'today_off')
    await fire('restDue', { kind: 'water', tired: false })
    await fire('restDue', { kind: 'night', tired: false })
    expect(cards(w)).toHaveLength(1)
    await cards(w)[0]!.trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(mocks.restAction).toHaveBeenLastCalledWith('night', 'later')
    expect(cards(w)).toHaveLength(0)
  })

  it('疲惫时护眼换文案', async () => {
    const w = await mountCards()
    await fire('restDue', { kind: 'eye', tired: true })
    expect(cards(w)[0]!.text()).toContain(t('rest.eye_tired'))
  })

  describe('护眼倒计时', () => {
    beforeEach(() =>
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] }),
    )
    afterEach(() => vi.useRealTimers())

    it('点“已完成”先倒数 20 秒，再显示致谢 2 秒，然后才交给后端', async () => {
      const w = await mountCards()
      await fire('restDue', { kind: 'eye', tired: false })
      await cards(w)[0]!.find('button.primary').trigger('click')
      await flushPromises()
      expect(w.find('[role=timer]').text()).toBe('20')
      expect(buttons(cards(w)[0]!)).toEqual([])
      await vi.advanceTimersByTimeAsync(19_000)
      expect(w.find('[role=timer]').text()).toBe('1')
      await vi.advanceTimersByTimeAsync(1_000)
      expect(w.find('[role=timer]').text()).toBe(t('rest.eye_done'))
      expect(mocks.restAction).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(2_000)
      await flushPromises()
      expect(mocks.restAction).toHaveBeenCalledWith('eye', 'done')
      expect(cards(w)).toHaveLength(0)
    })
  })
})

describe('自评回应、晚间小结、周信提示', () => {
  const comfort = (trigger: ComfortNew['trigger']): ComfortNew => ({
    id: 3,
    text: '有点累的时候，慢一点也没关系。',
    source: 'template',
    ai_generated: false,
    trigger,
  })

  it('负面自评后的回应：卡片给“和晴晴聊聊”，主动关怀不出卡片（FR-STA-10 第 2 条）', async () => {
    const w = await mountCards()
    await fire('comfortNew', comfort('auto'))
    expect(cards(w)).toHaveLength(0)
    await fire('comfortNew', comfort('self_report'))
    const card = cards(w)[0]!
    expect(card.find('.title').text()).toBe(t('cards.self_reply_title'))
    expect(buttons(card)).toEqual(['和晴晴聊聊', '先不用'])
    await card.find('button.primary').trigger('click')
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('chat')
    expect(cards(w)).toHaveLength(0)
  })

  it('晚间小结：统计、色带、结束语（不加 AI 标识），可以去看今天的看板', async () => {
    const summary: EveningSummary = {
      date: '2026-10-09',
      typing_min: 252,
      water: 3,
      rests_done: 5,
      rests_due: 7,
      schedules_done: 2,
      todos_done: 3,
      band: ['fluent', 'hesitant', 'fluent'],
      line: '今天辛苦了，晚上好好休息。',
    }
    const w = await mountCards()
    await fire('reviewEvening', summary)
    const card = cards(w)[0]!
    expect(card.find('.title').text()).toBe('🌙 今天的小结')
    expect(card.text()).toContain('⌨ 打字 4 小时 12 分')
    expect(card.text()).toContain('👀 休息 5/7 次')
    expect(card.find('.band').attributes('aria-label')).toBe('今天的天气：晴、多云、晴')
    expect(card.text()).toContain('今天辛苦了，晚上好好休息。')
    expect(card.find('.tag').exists()).toBe(false)
    await card.find('button.primary').trigger('click')
    await flushPromises()
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:dashboard-nav', { page: 'today' })
  })

  it('周信：AI 写的带标识；“等会儿再看”在一句话区留提示', async () => {
    const letter: LetterNew = { id: 4, week_start: '2026-10-05', ai_generated: true }
    const w = await mountCards()
    await fire('letterNew', letter)
    const card = cards(w)[0]!
    expect(card.find('.tag').text()).toBe('AI 生成')
    await card.findAll('.actions button')[1]!.trigger('click')
    await flushPromises()
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:review-hint', { letter: 4 })
    expect(cards(w)).toHaveLength(0)
  })

  it('最多同时 2 张，其余排队，按优先级', async () => {
    const w = await mountCards()
    await fire('letterNew', { id: 4, week_start: '2026-10-05', ai_generated: false })
    await fire('restDue', { kind: 'eye', tired: false })
    await fire('comfortNew', comfort('self_report'))
    expect(cards(w).map((c) => c.attributes('aria-label'))).toEqual([
      t('cards.self_reply_title'),
      t('notify.rest_title'),
    ])
    expect(w.find('.more').text()).toBe('后面还有 1 张')
  })
})
