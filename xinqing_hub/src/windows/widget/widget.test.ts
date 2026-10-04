import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { Explanation, StatusSnapshot } from '@/api'
import { t } from '@/i18n'

type Item = { id: string; text: string; action: () => void }
const mocks = vi.hoisted(() => ({
  snapshot: null as unknown as StatusSnapshot,
  openWindow: vi.fn(),
  pauseSet: vi.fn(),
  hide: vi.fn(),
  setAlwaysOnTop: vi.fn(),
  popup: vi.fn(),
  menuItems: [] as Item[],
  stateExplain: vi.fn(),
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
  },
  events: {
    statusChanged: { listen: async () => () => {} },
    settingsChanged: { listen: async () => () => {} },
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  LogicalPosition: class {
    constructor(
      public x: number,
      public y: number,
    ) {}
  },
  getCurrentWindow: () => ({ hide: mocks.hide, setAlwaysOnTop: mocks.setAlwaysOnTop }),
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
    for (const f of [mocks.openWindow, mocks.pauseSet, mocks.hide, mocks.popup])
      f.mockReset().mockReturnValue(ok())
    mocks.setAlwaysOnTop.mockReset().mockResolvedValue(undefined)
    mocks.menuItems = []
    mocks.stateExplain.mockReset().mockReturnValue(ok(null))
  })

  it('Shift+F10 在左上角弹出右键菜单（FR-WGT-06）', async () => {
    const w = await mountWidget()
    await w.find('main').trigger('keydown', { key: 'F10', shiftKey: true })
    await flushPromises()
    expect(mocks.menuItems.map((i) => i.text)).toEqual(['暂停感知', '打开看板', '设置', '隐藏小组件'])
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
    mocks.menuItems[0]!.action()
    await flushPromises()
    expect(mocks.menuItems[0]!.text).toBe('恢复感知')
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
    prob: 85,
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
    await w.find('.status').trigger('blur')
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
})
