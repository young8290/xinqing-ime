import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type {
  ComfortItem,
  ComfortNew,
  Explanation,
  ScheduleItem,
  SelfReportChanged,
  SelfReportItem,
  StatusSnapshot,
} from '@/api'
import { t } from '@/i18n'

type Item = { id: string; text: string; action: () => void }
const mocks = vi.hoisted(() => ({
  snapshot: null as unknown as StatusSnapshot,
  openWindow: vi.fn(),
  pauseSet: vi.fn(),
  hide: vi.fn(),
  show: vi.fn(),
  setAlwaysOnTop: vi.fn(),
  popup: vi.fn(),
  menuItems: [] as Item[],
  stateExplain: vi.fn(),
  submitFeedback: vi.fn(),
  emitStatus: null as null | ((s: StatusSnapshot) => void),
  selfReportSet: vi.fn(),
  selfReportList: [] as SelfReportItem[],
  emitSelfReport: null as null | ((e: SelfReportChanged) => void),
  comfortList: [] as ComfortItem[],
  comfortFeedback: vi.fn(),
  emitComfort: null as null | ((e: ComfortNew) => void),
  emitCareReduced: null as null | (() => void),
  emitSafetyInvite: null as null | (() => void),
  emitTypo: null as null | (() => void),
  emitInvite: null as null | (() => void),
  safetyOpen: vi.fn(),
  researchDismiss: vi.fn(),
  dbRebuilt: false,
  demo: false,
  settings: {} as Record<string, unknown>,
  typingMin: 0,
  schedules: [] as ScheduleItem[],
  pendingSchedules: [] as ScheduleItem[],
  openTodos: 0,
  busListeners: {} as Record<string, (e: { payload: unknown }) => void>,
  busEmit: vi.fn(),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    getStatus: () => ok(mocks.snapshot),
    settingsGet: (key: string) => ok(key in mocks.settings ? mocks.settings[key] : key === 'widget.topmost'),
    openWindow: mocks.openWindow,
    pauseSet: mocks.pauseSet,
    stateExplain: mocks.stateExplain,
    submitFeedback: mocks.submitFeedback,
    selfReportSet: mocks.selfReportSet,
    selfReportList: () => ok(mocks.selfReportList),
    comfortList: () => ok(mocks.comfortList),
    comfortFeedback: mocks.comfortFeedback,
    dbRebuiltTake: () => Promise.resolve(mocks.dbRebuilt),
    demoStatus: () => ok({ enabled: mocks.demo, speed: mocks.demo ? 60 : 1 }),
    safetyOpen: mocks.safetyOpen,
    researchDismiss: mocks.researchDismiss,
    dayStats: () =>
      ok({
        date: '',
        typing_min: mocks.typingMin,
        rests_due: 0,
        rests_done: 0,
        water: 0,
        comforts: 0,
        dominant: null,
        timeline: [],
      }),
    scheduleList: (status: string) => ok(status === 'pending' ? mocks.pendingSchedules : mocks.schedules),
    todoList: (status: string) =>
      ok(Array.from({ length: status === 'open' ? mocks.openTodos : 0 }, (_, i) => ({ id: i }))),
  },
  events: {
    statusChanged: {
      listen: async (cb: (e: { payload: StatusSnapshot }) => void) => {
        mocks.emitStatus = (payload) => cb({ payload })
        return () => {}
      },
    },
    settingsChanged: { listen: async () => () => {} },
    selfReportChanged: {
      listen: async (cb: (e: { payload: SelfReportChanged }) => void) => {
        mocks.emitSelfReport = (payload) => cb({ payload })
        return () => {}
      },
    },
    comfortNew: {
      listen: async (cb: (e: { payload: ComfortNew }) => void) => {
        mocks.emitComfort = (payload) => cb({ payload })
        return () => {}
      },
    },
    careReduced: {
      listen: async (cb: () => void) => {
        mocks.emitCareReduced = cb
        return () => {}
      },
    },
    safetyInvite: {
      listen: async (cb: () => void) => {
        mocks.emitSafetyInvite = cb
        return () => {}
      },
    },
    moodTypo: {
      listen: async (cb: () => void) => {
        mocks.emitTypo = cb
        return () => {}
      },
    },
    researchInvite: {
      listen: async (cb: () => void) => {
        mocks.emitInvite = cb
        return () => {}
      },
    },
    reminderDue: { listen: async () => () => {} },
    todoReady: { listen: async () => () => {} },
  },
}))
vi.mock('@tauri-apps/api/event', () => ({
  emit: mocks.busEmit,
  listen: async (name: string, cb: (e: { payload: unknown }) => void) => {
    mocks.busListeners[name] = cb
    return () => {}
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  LogicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
  getCurrentWindow: () => ({ hide: mocks.hide, show: mocks.show, setAlwaysOnTop: mocks.setAlwaysOnTop }),
  // 定位需要的接口不给：useWidgetWindow 会失败并只打警告，界面照常显示
}))
vi.mock('@tauri-apps/api/menu', () => ({
  Menu: {
    new: async ({ items }: { items: Item[] }) => {
      mocks.menuItems = items
      return { popup: mocks.popup }
    },
  },
}))

const { default: App } = await import('./App.vue')

// 新加的来源默认都是空的，各用例按需打开
beforeEach(() => {
  mocks.comfortList = []
  mocks.comfortFeedback.mockReset().mockReturnValue(ok())
  mocks.safetyOpen.mockReset().mockReturnValue(ok(5))
  mocks.researchDismiss.mockReset().mockReturnValue(ok())
  mocks.dbRebuilt = false
  mocks.demo = false
  mocks.settings = {}
  mocks.typingMin = 0
  mocks.schedules = []
  mocks.pendingSchedules = []
  mocks.openTodos = 0
  mocks.busListeners = {}
  mocks.busEmit.mockReset().mockResolvedValue(undefined)
  try {
    localStorage.clear()
  } catch {
    // jsdom 里总能用
  }
})

async function mountWidget() {
  const w = mount(App)
  await flushPromises()
  return w
}

describe('小组件', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.snapshot = {
      state: 'hesitant',
      weather: 'cloudy',
      prob: null,
      offline: false,
      paused: false,
      connected: true,
      baseline_progress: 100,
    }
    mocks.show.mockReset().mockResolvedValue(undefined)
    for (const f of [mocks.openWindow, mocks.pauseSet, mocks.hide, mocks.popup])
      f.mockReset().mockReturnValue(ok())
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.menuItems = []
    mocks.stateExplain.mockReset().mockReturnValue(ok(null))
  })

  it('外壳建窗口时不显示，前端定位后再显示；定位失败也照样显示（FR-WGT-01）', async () => {
    await mountWidget()
    expect(mocks.show).toHaveBeenCalledTimes(1)
  })

  it('Shift+F10 在左上角弹出右键菜单（FR-WGT-06）', async () => {
    const w = await mountWidget()
    await w.find('main').trigger('keydown', { key: 'F10', shiftKey: true })
    await flushPromises()
    expect(mocks.menuItems.map((i) => i.text)).toEqual([
      '我现在…',
      '暂停感知',
      '打开看板',
      '待确认日程与待办',
      '设置',
      '隐藏小组件',
    ])
    expect(mocks.popup).toHaveBeenCalledWith(expect.objectContaining({ x: 16, y: 16 }))
  })

  it('右键在鼠标处弹出；菜单项调用对应命令', async () => {
    const w = await mountWidget()
    await w.find('main').trigger('contextmenu')
    await flushPromises()
    expect(mocks.popup).toHaveBeenCalledWith(undefined)
    const pick = (id: string) => mocks.menuItems.find((i) => i.id === id)!.action()
    pick('pause')
    pick('dashboard')
    pick('hide')
    await flushPromises()
    expect(mocks.pauseSet).toHaveBeenCalledWith(true)
    expect(mocks.openWindow).toHaveBeenCalledWith('dashboard')
    expect(mocks.hide).toHaveBeenCalled()
  })

  it('已暂停时菜单给“恢复感知”', async () => {
    mocks.snapshot.paused = true
    const w = await mountWidget()
    await w.find('main').trigger('contextmenu')
    await flushPromises()
    mocks.menuItems[1]!.action()
    await flushPromises()
    expect(mocks.menuItems[1]!.text).toBe('恢复感知')
    expect(mocks.pauseSet).toHaveBeenCalledWith(false)
  })

  it('Enter 打开对话；置顶跟随设置', async () => {
    const w = await mountWidget()
    await w.find('main').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('chat')
    expect(mocks.setAlwaysOnTop).toHaveBeenLastCalledWith(true)
  })

  it('命令失败时一句话区显示可读的提示，不显示错误码', async () => {
    mocks.openWindow.mockReturnValue(
      ok().then(() => ({ status: 'error', error: { code: 'x', message_key: 'nope' } })),
    )
    const w = await mountWidget()
    await w.find('main').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(w.find('.message').text()).toBe(t('error.generic'))
  })
})

describe('悬停状态行显示解释（FR-WGT-06、FR-STA-09）', () => {
  const explanation: Explanation = {
    state: 'hesitant',
    prob_pct: 85,
    signals: [
      { kind: 'pause', value: 2 },
      { kind: 'abandon', value: null },
    ],
    source: 'jev',
    cold_start: false,
  }

  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.snapshot = {
      state: 'hesitant',
      weather: 'cloudy',
      prob: 0.85,
      offline: false,
      paused: false,
      connected: true,
      baseline_progress: 100,
    }
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.stateExplain.mockReset().mockReturnValue(ok(explanation))
  })

  async function focusStatus() {
    const w = await mountWidget()
    await w.find('.status').trigger('focus')
    await flushPromises()
    return w
  }

  it('键盘聚焦状态行：取当前解释，每条说明一行，来源弱化', async () => {
    const w = await focusStatus()
    expect(mocks.stateExplain).toHaveBeenCalledWith(null)
    const panel = w.find('[role=tooltip]')
    expect(panel.find('.header').text()).toBe('看起来有点犹豫（可能性 85%）')
    expect(panel.findAll('li').map((li) => li.text())).toEqual(['句子中间停顿了 2 次', '有一段话打了又删'])
    expect(panel.find('.footer').text()).toBe('AI 根据打字节奏判断，可能不准')
    expect(w.find('.status').attributes('aria-describedby')).toBe(panel.attributes('id'))
  })

  it('状态行本身不显示百分比（DS-COPY-02）', async () => {
    const w = await mountWidget()
    expect(w.find('.status').text()).not.toContain('85')
  })

  it('Esc 和失焦都会收起', async () => {
    const w = await focusStatus()
    await w.find('main').trigger('keydown', { key: 'Escape' })
    expect(w.find('[role=tooltip]').exists()).toBe(false)
    await w.find('.status').trigger('focus')
    await flushPromises()
    await w.find('.status').trigger('focusout', { relatedTarget: null })
    expect(w.find('[role=tooltip]').exists()).toBe(false)
  })

  it('鼠标停留 300 ms 才打开，移开后收起', async () => {
    vi.useFakeTimers()
    try {
      const w = await mountWidget()
      await w.find('.status').trigger('pointerenter')
      await vi.advanceTimersByTimeAsync(200)
      expect(w.find('[role=tooltip]').exists()).toBe(false)
      await vi.advanceTimersByTimeAsync(150)
      await flushPromises()
      expect(w.find('[role=tooltip]').exists()).toBe(true)
      await w.find('.status').trigger('pointerleave')
      await vi.advanceTimersByTimeAsync(200)
      expect(w.find('[role=tooltip]').exists()).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('后端的解释属于别的状态（刚切换）：不用它，只显示标题', async () => {
    mocks.stateExplain.mockReturnValue(ok({ ...explanation, state: 'low' }))
    const w = await focusStatus()
    expect(w.find('[role=tooltip] .header').text()).toBe('看起来有点犹豫（可能性 85%）')
    expect(w.find('[role=tooltip] li').exists()).toBe(false)
  })

  it('还没有解释、也没有可能性：不弹面板', async () => {
    mocks.snapshot.prob = null
    mocks.stateExplain.mockReturnValue(ok(null))
    const w = await focusStatus()
    expect(w.find('[role=tooltip]').exists()).toBe(false)
  })

  it('暂停感知时没有解释，也不去取', async () => {
    mocks.snapshot.paused = true
    const w = await focusStatus()
    expect(w.find('[role=tooltip]').exists()).toBe(false)
    expect(mocks.stateExplain).not.toHaveBeenCalled()
  })

  describe('“准 / 不准”（FR-STA-07）', () => {
    beforeEach(() => {
      mocks.submitFeedback.mockReset().mockReturnValue(ok())
      mocks.openWindow.mockReset().mockReturnValue(ok())
    })

    const buttons = (w: Awaited<ReturnType<typeof focusStatus>>) =>
      w.findAll('[role=tooltip] [role=group] button')

    it('和解释放在一起；点“不准”记到当前状态并致谢，不会打开对话', async () => {
      const w = await focusStatus()
      expect(buttons(w).map((b) => b.text())).toEqual(['准', '不准'])
      expect(w.find('[role=group]').attributes('aria-label')).toBe('这个判断准吗')
      await buttons(w)[1]!.trigger('pointerdown')
      await buttons(w)[1]!.trigger('pointerup')
      await buttons(w)[1]!.trigger('click')
      await flushPromises()
      expect(mocks.submitFeedback).toHaveBeenCalledWith('mood_state', null, 'unfit')
      expect(w.find('[role=tooltip] [role=status]').text()).toBe('谢谢，我记下了')
      expect(buttons(w)).toHaveLength(0)
      expect(mocks.openWindow).not.toHaveBeenCalled()
    })

    it('键盘投票：按钮换成致谢后焦点留在面板里，面板不收起', async () => {
      const w = await mountWidget()
      document.body.appendChild(w.element)
      await w.find('.status').trigger('focus')
      await flushPromises()
      ;(buttons(w)[0]!.element as HTMLButtonElement).focus()
      await buttons(w)[0]!.trigger('click')
      await flushPromises()
      expect(w.find('[role=tooltip]').exists()).toBe(true)
      expect(document.activeElement).toBe(w.find('[role=tooltip] [role=status]').element)
      w.element.remove()
    })

    it('焦点从状态行移到按钮上，面板不收起', async () => {
      const w = await focusStatus()
      await w.find('.status').trigger('focusout', { relatedTarget: buttons(w)[0]!.element })
      expect(w.find('[role=tooltip]').exists()).toBe(true)
    })

    it('状态变了再重新问', async () => {
      const w = await focusStatus()
      await buttons(w)[0]!.trigger('click')
      await flushPromises()
      expect(mocks.submitFeedback).toHaveBeenCalledWith('mood_state', null, 'fit')
      mocks.snapshot = { ...mocks.snapshot, state: 'tired', weather: 'night' }
      mocks.emitStatus!(mocks.snapshot)
      mocks.stateExplain.mockReturnValue(ok({ ...explanation, state: 'tired' }))
      await flushPromises()
      expect(w.find('[role=tooltip]').exists()).toBe(false)
      await w.find('.status').trigger('focus')
      await flushPromises()
      expect(buttons(w)).toHaveLength(2)
    })

    it('提交失败：一句话区给可读提示，按钮还在', async () => {
      mocks.submitFeedback.mockReturnValue(
        ok().then(() => ({ status: 'error', error: { code: 'x', message_key: 'nope' } })),
      )
      const w = await focusStatus()
      await buttons(w)[1]!.trigger('click')
      await flushPromises()
      expect(w.find('.message').text()).toBe(t('error.generic'))
      expect(buttons(w)).toHaveLength(2)
    })
  })
})

describe('“我现在…”自评（FR-STA-10、FR-WGT-06）', () => {
  const NOW = 1_700_000_000_000
  const HOUR = 60 * 60 * 1000

  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.snapshot = {
      state: 'fluent',
      weather: 'sunny',
      prob: 0.9,
      offline: false,
      paused: false,
      connected: true,
      baseline_progress: 100,
    }
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.show.mockReset().mockResolvedValue(undefined)
    mocks.openWindow.mockReset().mockReturnValue(ok())
    mocks.stateExplain.mockReset().mockReturnValue(ok(null))
    mocks.selfReportSet.mockReset().mockReturnValue(ok())
    mocks.selfReportList = []
    vi.useFakeTimers({ now: NOW, toFake: ['Date', 'setTimeout', 'clearTimeout'] })
  })

  afterEach(() => vi.useRealTimers())

  const status = (w: Awaited<ReturnType<typeof mountWidget>>) => w.find('.status').text()

  it('点状态行的 ✎ 打开面板：六个选项，备注选填；点选项就提交并关面板，不会打开对话', async () => {
    const w = await mountWidget()
    await w.find('button.pen').trigger('click')
    const dialog = w.find('[role=dialog]')
    expect(dialog.attributes('aria-label')).toBe('我现在…')
    expect(dialog.findAll('button.option').map((b) => b.text())).toEqual([
      '☀️ 挺好',
      '⛅ 有点犹豫',
      '🌧 有点低落',
      '⛈ 有点烦',
      '🌙 有点累',
      '🤷 说不上来',
    ])
    expect(dialog.find('input').attributes('maxlength')).toBe('50')
    await dialog.find('input').setValue('  加班到现在  ')
    await dialog.find('button.option[data-weather=night]').trigger('click')
    await flushPromises()
    expect(mocks.selfReportSet).toHaveBeenCalledWith('night', '加班到现在')
    expect(w.find('[role=dialog]').exists()).toBe(false)
    expect(mocks.openWindow).not.toHaveBeenCalled()
  })

  it('不填备注时传 null；右键菜单的“我现在…”也打开面板', async () => {
    const w = await mountWidget()
    await w.find('main').trigger('contextmenu')
    await flushPromises()
    mocks.menuItems[0]!.action()
    await flushPromises()
    await w.find('button.option[data-weather=sunny]').trigger('click')
    await flushPromises()
    expect(mocks.selfReportSet).toHaveBeenCalledWith('sunny', null)
  })

  it('面板里 Esc 关闭、Enter 不会打开对话', async () => {
    const w = await mountWidget()
    await w.find('button.pen').trigger('click')
    await w.find('[role=dialog] input').trigger('keydown', { key: 'Enter' })
    expect(mocks.openWindow).not.toHaveBeenCalled()
    await w.find('[role=dialog]').trigger('keydown', { key: 'Escape' })
    expect(w.find('[role=dialog]').exists()).toBe(false)
  })

  it('收到 self_report:changed：状态行显示“你说的”，到期自动回到自动判断', async () => {
    const w = await mountWidget()
    mocks.emitSelfReport!({ weather: 'night', until_ts: NOW + HOUR })
    await flushPromises()
    expect(status(w)).toBe('🌙 你说的：有点累')
    await vi.advanceTimersByTimeAsync(HOUR)
    expect(status(w)).toBe('☀️ 晴 · 看起来挺顺的')
  })

  it('“说不上来”不覆盖显示', async () => {
    const w = await mountWidget()
    mocks.emitSelfReport!({ weather: 'unsure', until_ts: NOW })
    await flushPromises()
    expect(status(w)).toBe('☀️ 晴 · 看起来挺顺的')
  })

  it('窗口打开时由当天的自评记录接上覆盖期', async () => {
    mocks.selfReportList = [
      { id: 3, ts: NOW - 10 * 60 * 1000, weather: 'rain', note: null, auto_state: 'fluent' },
    ]
    const w = await mountWidget()
    expect(status(w)).toBe('🌧 你说的：有点低落')
  })

  it('覆盖期内悬停状态行不弹解释（显示的是用户自己说的）', async () => {
    const w = await mountWidget()
    mocks.emitSelfReport!({ weather: 'night', until_ts: NOW + HOUR })
    await flushPromises()
    await w.find('.status').trigger('focus')
    await flushPromises()
    expect(w.find('[role=tooltip]').exists()).toBe(false)
  })

  it('提交失败：一句话区给可读提示，面板留着可以重试', async () => {
    mocks.selfReportSet.mockReturnValue(
      ok().then(() => ({ status: 'error', error: { code: 'x', message_key: 'nope' } })),
    )
    const w = await mountWidget()
    await w.find('button.pen').trigger('click')
    await w.find('button.option[data-weather=rain]').trigger('click')
    await flushPromises()
    expect(w.find('.message').text()).toBe(t('error.generic'))
    expect(w.find('[role=dialog]').exists()).toBe(true)
  })
})

describe('一句话区（FR-WGT-04、FR-CMF-04/05）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.snapshot = {
      state: 'fluent',
      weather: 'sunny',
      prob: 0.9,
      offline: false,
      paused: false,
      connected: true,
      baseline_progress: 100,
    }
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.show.mockReset().mockResolvedValue(undefined)
    mocks.openWindow.mockReset().mockReturnValue(ok())
    mocks.stateExplain.mockReset().mockReturnValue(ok(null))
  })

  const comfort = (p: Partial<ComfortNew> = {}): ComfortNew => ({
    id: 11,
    text: '想说的话不急着说完，慢慢来。',
    source: 'llm',
    ai_generated: true,
    trigger: 'auto',
    ...p,
  })

  it('没有消息时是空闲问候', async () => {
    const w = await mountWidget()
    expect(w.find('.message').text()).toBe(t('greeting.idle'))
    expect(w.find('.ai-tag').exists()).toBe(false)
  })

  it('打开时用今天最后一句暖心话做“今日一句”，AI 写的带标识', async () => {
    mocks.comfortList = [
      { id: 1, ts: 1, text: '早上的', ai_generated: false, trigger: 'auto', feedback: null },
      { id: 2, ts: 2, text: '刚才那句', ai_generated: true, trigger: 'auto', feedback: null },
    ]
    const w = await mountWidget()
    expect(w.find('.message').text()).toBe('刚才那句')
    expect(w.find('.ai-tag').text()).toBe('AI 生成')
  })

  describe('新的暖心话', () => {
    beforeEach(() =>
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] }),
    )
    afterEach(() => vi.useRealTimers())

    it('逐字出现（每字 40 ms），模板句不带 AI 标识', async () => {
      const w = await mountWidget()
      mocks.emitComfort!(comfort({ text: '慢慢来。', ai_generated: false, source: 'template' }))
      await flushPromises()
      expect(w.find('.message').text()).toBe('')
      await vi.advanceTimersByTimeAsync(80)
      expect(w.find('.message').text()).toBe('慢慢')
      await vi.advanceTimersByTimeAsync(200)
      expect(w.find('.message').text()).toBe('慢慢来。')
      expect(w.find('.ai-tag').exists()).toBe(false)
    })

    it('2 小时后淡出为今日一句：同一句话还在，问候不会盖过它', async () => {
      const w = await mountWidget()
      mocks.emitComfort!(comfort())
      await vi.advanceTimersByTimeAsync(2 * 60 * 60 * 1000 + 1000)
      expect(w.find('.message').text()).toBe(comfort().text)
      expect(w.find('.ai-tag').exists()).toBe(true)
    })
  })

  it('悬停时的 👍 / 👎 / 🔕：记到这句话上并致谢（FR-CMF-05）', async () => {
    const w = await mountWidget()
    mocks.emitComfort!(comfort())
    await flushPromises()
    const group = w.find('[role=group][aria-label="这句话怎么样"]')
    expect(group.findAll('button').map((b) => b.attributes('aria-label'))).toEqual([
      '👍 有用',
      '👎 不合适',
      '🔕 今天先别说了',
    ])
    await group.find('button[data-verdict=mute]').trigger('click')
    await flushPromises()
    expect(mocks.comfortFeedback).toHaveBeenCalledWith(11, 'mute')
    expect(w.find('.thanks').text()).toBe(t('widget.comfort.muted'))
    expect(mocks.openWindow).not.toHaveBeenCalled()
  })

  it('降档时告诉用户“我会少打扰你一些”', async () => {
    const w = await mountWidget()
    mocks.emitCareReduced!()
    await flushPromises()
    expect(w.find('.message').text()).toBe(t('widget.care_reduced'))
  })

  it('改写的求助入口优先级最高，点了开安全模式对话并收起（ADR 0028）', async () => {
    const w = await mountWidget()
    mocks.emitComfort!(comfort())
    mocks.emitSafetyInvite!()
    await flushPromises()
    const link = w.find('button.message')
    expect(link.text()).toBe(t('rewrite.crisis_bubble'))
    await link.trigger('click')
    await flushPromises()
    expect(mocks.safetyOpen).toHaveBeenCalled()
    expect(mocks.openWindow).not.toHaveBeenCalled()
    expect(w.find('button.message').exists()).toBe(false)
  })

  it('卡片层转来的周信提示：点了打开看板信箱', async () => {
    const w = await mountWidget()
    mocks.busListeners['xq:review-hint']!({ payload: { letter: 4 } })
    await flushPromises()
    await w.find('button.message').trigger('click')
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('dashboard')
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:dashboard-nav', { page: 'mailbox', letter: 4 })
  })

  it('数据库刚重建过：提示一次（FR-DAT-01）', async () => {
    mocks.dbRebuilt = true
    const w = await mountWidget()
    expect(w.find('.message').text()).toBe(t('error.db_rebuilt'))
  })
})

describe('底栏、角标与其他（FR-WGT-05、FR-DMO-03/04）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.snapshot = {
      state: 'fluent',
      weather: 'sunny',
      prob: 0.9,
      offline: false,
      paused: false,
      connected: true,
      baseline_progress: 100,
    }
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.show.mockReset().mockResolvedValue(undefined)
    mocks.openWindow.mockReset().mockReturnValue(ok())
    mocks.stateExplain.mockReset().mockReturnValue(ok(null))
    mocks.menuItems = []
    mocks.popup.mockReset().mockReturnValue(ok())
  })

  it('底栏：今日输入时长与待办数，点了打开看板“日程与待办”', async () => {
    mocks.typingMin = 102
    mocks.openTodos = 3
    const w = await mountWidget()
    expect(w.find('.footer-left').text()).toBe('⌨ 今日 1 小时 42 分')
    const right = w.find('.footer-right')
    expect(right.text()).toBe('✅ 3 件待办')
    await right.trigger('click')
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('dashboard')
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:dashboard-nav', { page: 'schedule' })
  })

  it('没有日程也没有待办时右边不显示', async () => {
    const w = await mountWidget()
    expect(w.find('.footer-right').exists()).toBe(false)
  })

  it('菜单里的待确认日程与待办带数量，点了打开日程页', async () => {
    mocks.pendingSchedules = [{ id: 1 } as ScheduleItem, { id: 2 } as ScheduleItem]
    const w = await mountWidget()
    await w.find('main').trigger('contextmenu')
    await flushPromises()
    const item = mocks.menuItems.find((i) => i.id === 'pending')!
    expect(item.text).toBe('待确认日程与待办（2）')
    item.action()
    await flushPromises()
    expect(mocks.busEmit).toHaveBeenCalledWith('xq:dashboard-nav', { page: 'schedule' })
  })

  it('研究模式开着（有编号）时显示标识；演示模式也显示', async () => {
    mocks.settings = { 'research.enabled': true, 'research.id': 'P07' }
    mocks.demo = true
    const w = await mountWidget()
    const badges = w.findAll('.badge').map((b) => b.text())
    expect(badges).toEqual([t('widget.demo_badge'), t('widget.research_badge')])
  })

  it('研究模式开关开着但没填编号：不算研究模式', async () => {
    mocks.settings = { 'research.enabled': true, 'research.id': '' }
    const w = await mountWidget()
    expect(w.findAll('.badge')).toHaveLength(0)
  })

  it('研究模式的邀请：弹出“我现在…”面板，换标题，可以跳过', async () => {
    const w = await mountWidget()
    mocks.emitInvite!()
    await flushPromises()
    const dialog = w.find('[role=dialog]')
    expect(dialog.attributes('aria-label')).toBe(t('self_report.invite_title'))
    await dialog.find('button.skip').trigger('click')
    await flushPromises()
    expect(mocks.researchDismiss).toHaveBeenCalled()
    expect(w.find('[role=dialog]').exists()).toBe(false)
  })

  it('卡片层有卡片时告诉小组件（贴边隐藏不收起）', async () => {
    await mountWidget()
    expect(mocks.busListeners['xq:cards']).toBeTypeOf('function')
  })
})
