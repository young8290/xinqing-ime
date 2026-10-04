import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { ConsentItem, ConsentState } from '@/api'

const ITEMS: ConsentItem[] = ['sense', 'jev_features', 'schedule', 'llm_summary', 'enhanced', 'rewrite']
const mocks = vi.hoisted(() => ({
  granted: new Set<string>(),
  consentSet: vi.fn(),
  openWindow: vi.fn(),
  close: vi.fn(),
}))
const state = (): ConsentState => ({
  policy_ver: 1,
  items: ITEMS.map((item) => ({ item, granted: mocks.granted.has(item) })),
})

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    consentGet: async () => ({ status: 'ok', data: state() }),
    consentSet: mocks.consentSet,
    openWindow: mocks.openWindow,
  },
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: mocks.close }) }))

const { default: App } = await import('./App.vue')

async function toConsentPage() {
  const w = mount(App)
  await flushPromises()
  await w.findAll('button')[0]!.trigger('click') // 我已年满 18 周岁
  await w.find('button.primary').trigger('click') // 告知页 → 下一步
  return w
}

describe('首次引导', () => {
  beforeEach(() => {
    mocks.granted.clear()
    mocks.consentSet.mockReset().mockImplementation(async (item: string, on: boolean) => {
      if (on) mocks.granted.add(item)
      else mocks.granted.delete(item)
      return { status: 'ok', data: state() }
    })
    mocks.openWindow.mockReset().mockResolvedValue({ status: 'ok', data: null })
    mocks.close.mockReset()
  })

  it('同意项默认都不勾选，增强模式不在主列表里（FR-ONB-04）', async () => {
    const w = await toConsentPage()
    const boxes = w.findAll<HTMLInputElement>('input[type=checkbox]')
    expect(boxes).toHaveLength(5)
    expect(boxes.every((b) => !b.element.checked)).toBe(true)
    expect(w.find('[data-item=enhanced]').exists()).toBe(false)
  })

  it('不勾选 ① 不能继续；勾选后落库并可继续', async () => {
    const w = await toConsentPage()
    const next = () => w.find('button.primary')
    expect(next().attributes('disabled')).toBeDefined()
    await w.find('[data-item=sense] input').trigger('change')
    await flushPromises()
    expect(mocks.consentSet).toHaveBeenCalledWith('sense', true)
    expect(next().attributes('disabled')).toBeUndefined()
  })

  it('完成后打开小组件并关闭引导窗口', async () => {
    const w = await toConsentPage()
    await w.find('[data-item=sense] input').trigger('change')
    await flushPromises()
    await w.find('button.primary').trigger('click') // → 增强模式页
    expect(w.find('[data-item=enhanced] strong').exists()).toBe(true)
    await w.find('button.primary').trigger('click') // 开始使用
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('widget')
    expect(mocks.close).toHaveBeenCalled()
  })

  it('未满 18 周岁：说明后只能关闭，不进入同意页（FR-ONB-02）', async () => {
    const w = mount(App)
    await flushPromises()
    await w.findAll('button')[1]!.trigger('click')
    expect(w.text()).toContain('心晴的陪伴功能暂时只面向成年人')
    expect(w.find('input[type=checkbox]').exists()).toBe(false)
    await w.find('button').trigger('click')
    expect(mocks.close).toHaveBeenCalled()
  })
})
