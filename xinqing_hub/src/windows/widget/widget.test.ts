import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { Explanation, RestDue, SelfReportChanged, SelfReportItem, StatusSnapshot } from '@/api'
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
  restAction: vi.fn(),
  emitRest: null as null | ((e: RestDue) => void),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    getStatus: () => ok(mocks.snapshot),
    settingsGet: (key: string) => ok(key === 'widget.topmost'),
    openWindow: mocks.openWindow,
    pauseSet: mocks.pauseSet,
    stateExplain: mocks.stateExplain,
    submitFeedback: mocks.submitFeedback,
    selfReportSet: mocks.selfReportSet,
    selfReportList: () => ok(mocks.selfReportList),
    restAction: mocks.restAction,
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
    restDue: {
      listen: async (cb: (e: { payload: RestDue }) => void) => {
        mocks.emitRest = (payload) => cb({ payload })
        return () => {}
      },
    },
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

describe('休息提醒卡片（FR-RST-02～06）', () => {
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
    mocks.restAction.mockReset().mockReturnValue(ok())
  })

  async function due(payload: RestDue) {
    const w = await mountWidget()
    mocks.emitRest!(payload)
    await flushPromises()
    return w
  }
  const buttons = (w: Awaited<ReturnType<typeof mountWidget>>) =>
    w.findAll('[role=alertdialog] button').map((b) => b.text())

  it('喝水：卡片给三个按钮，“已完成”叫“喝了”；点了就收起并交给后端，不会打开对话', async () => {
    const w = await due({ kind: 'water', tired: false })
    const card = w.find('[role=alertdialog]')
    expect(card.text()).toContain(t('rest.water'))
    expect(buttons(w)).toEqual([t('rest.btn_drank'), t('rest.btn_later'), t('rest.btn_today_off')])
    await card.findAll('button')[0]!.trigger('click')
    await flushPromises()
    expect(mocks.restAction).toHaveBeenCalledWith('water', 'done')
    expect(w.find('[role=alertdialog]').exists()).toBe(false)
    expect(mocks.openWindow).not.toHaveBeenCalled()
  })

  it('“今天不再提醒”与 Esc（等同 5 分钟后）', async () => {
    const w = await due({ kind: 'move', tired: false })
    await w.findAll('[role=alertdialog] button')[2]!.trigger('click')
    await flushPromises()
    expect(mocks.restAction).toHaveBeenLastCalledWith('move', 'today_off')
    mocks.emitRest!({ kind: 'night', tired: false })
    await flushPromises()
    await w.find('[role=alertdialog]').trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(mocks.restAction).toHaveBeenLastCalledWith('night', 'later')
    expect(w.find('[role=alertdialog]').exists()).toBe(false)
  })

  it('疲惫时护眼换文案', async () => {
    const w = await due({ kind: 'eye', tired: true })
    expect(w.find('[role=alertdialog]').text()).toContain(t('rest.eye_tired'))
  })

  describe('护眼倒计时', () => {
    beforeEach(() =>
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval'] }),
    )
    afterEach(() => vi.useRealTimers())

    it('点“已完成”先倒数 20 秒，再显示致谢 2 秒，然后才交给后端', async () => {
      const w = await due({ kind: 'eye', tired: false })
      await w.findAll('[role=alertdialog] button')[0]!.trigger('click')
      await flushPromises()
      expect(w.find('[role=timer]').text()).toBe('20')
      expect(buttons(w)).toEqual([])
      await vi.advanceTimersByTimeAsync(19_000)
      expect(w.find('[role=timer]').text()).toBe('1')
      await vi.advanceTimersByTimeAsync(1_000)
      expect(w.find('[role=timer]').text()).toBe(t('rest.eye_done'))
      expect(mocks.restAction).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(2_000)
      await flushPromises()
      expect(mocks.restAction).toHaveBeenCalledWith('eye', 'done')
      expect(w.find('[role=alertdialog]').exists()).toBe(false)
    })
  })
})
