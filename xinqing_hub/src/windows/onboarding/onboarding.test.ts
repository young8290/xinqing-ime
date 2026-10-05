import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import type { AiConfigView, ConsentItem, ConsentState } from '@/api'

const ITEMS: ConsentItem[] = ['sense', 'jev_features', 'schedule', 'llm_summary', 'enhanced', 'rewrite']
const mocks = vi.hoisted(() => ({
  granted: new Set<string>(),
  consentSet: vi.fn(),
  openWindow: vi.fn(),
  close: vi.fn(),
  aiConfigGet: vi.fn(),
  secretsSet: vi.fn(),
  aiTestConnection: vi.fn(),
  settings: {} as Record<string, unknown>,
  settingsSet: vi.fn(),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })
const NO_AI: AiConfigView = { source: 'none', jev: null, llm: null }
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
    aiConfigGet: mocks.aiConfigGet,
    secretsSet: mocks.secretsSet,
    aiTestConnection: mocks.aiTestConnection,
    settingsGet: async (key: string) => ok(mocks.settings[key]),
    settingsSet: mocks.settingsSet,
  },
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: mocks.close }) }))

const { default: App } = await import('./App.vue')

const mountApp = () => mount(App, { global: { plugins: [createPinia()] } })

async function toConsentPage() {
  const w = mountApp()
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
    mocks.aiConfigGet.mockReset().mockImplementation(() => ok(NO_AI))
    mocks.secretsSet.mockReset().mockImplementation(() => ok(NO_AI))
    mocks.aiTestConnection.mockReset().mockImplementation(() => ok([]))
    mocks.settings = {
      'care.level': 'normal',
      'rest.eye.enabled': true,
      'rest.water.enabled': true,
      'rest.move.enabled': true,
      'rest.night.enabled': false,
    }
    mocks.settingsSet.mockReset().mockImplementation(() => ok())
    localStorage.clear()
  })

  /** 勾 ① → 增强模式页 → AI 服务页 */
  async function toAiPage() {
    const w = await toConsentPage()
    await w.find('[data-item=sense] input').trigger('change')
    await flushPromises()
    await w.find('button.primary').trigger('click') // → 增强模式页
    await w.find('button.primary').trigger('click') // → AI 服务
    await flushPromises()
    return w
  }

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

  it('跳过 AI 服务、选完偏好：打开小组件、请晴晴打招呼，关闭引导窗口（FR-ONB-05）', async () => {
    const w = await toAiPage()
    await w.find('button.skip').trigger('click')
    await flushPromises()
    expect(mocks.secretsSet).not.toHaveBeenCalled()
    expect(w.find('[data-pref=care]').exists()).toBe(true)
    await w.find('button.primary').trigger('click') // 开始使用
    await flushPromises()
    expect(mocks.openWindow).toHaveBeenCalledWith('widget')
    expect(mocks.close).toHaveBeenCalled()
    expect(localStorage.getItem('xq.widget.greet')).toBe('1')
  })

  describe('AI 服务（FR-ONB-05 第 1 条）', () => {
    it('已保存的配置：地址填好，密钥只显示末 4 位、留空就不改', async () => {
      mocks.aiConfigGet.mockImplementation(() =>
        ok({
          source: 'saved',
          jev: { base_url: 'https://jev.example', key_tail: '••••a1b2', model: 'jev-latest' },
          llm: { base_url: 'https://llm.example', key_tail: null, models: ['m1', 'm2'] },
        }),
      )
      const w = await toAiPage()
      const jev = w.find('[data-service=jev]')
      expect((jev.find('input[type=url]').element as HTMLInputElement).value).toBe('https://jev.example')
      expect(jev.find('input[type=password]').attributes('placeholder')).toBe('已保存 ••••a1b2，留空就不改')
      // 没改什么，直接下一步不保存
      await w.find('button.primary').trigger('click')
      await flushPromises()
      expect(mocks.secretsSet).not.toHaveBeenCalled()
      expect(w.find('[data-pref=corner]').exists()).toBe(true)
    })

    it('填了地址和密钥：下一步时保存，模型沿用已有的（没有就留空用默认）', async () => {
      const w = await toAiPage()
      await w.find('[data-service=llm] input[type=url]').setValue(' https://llm.example ')
      await w.find('[data-service=llm] input[type=password]').setValue('sk-123')
      await w.find('button.primary').trigger('click')
      await flushPromises()
      expect(mocks.secretsSet).toHaveBeenCalledWith({
        jev: null,
        llm: { base_url: 'https://llm.example', api_key: 'sk-123', models: [] },
      })
      expect(w.find('[data-pref=corner]').exists()).toBe(true)
    })

    it('测试连接：先保存再测，逐个模型显示结果；Jev 说明没有测试接口', async () => {
      mocks.aiTestConnection.mockImplementation(() =>
        ok([
          { model: 'm1', ok: true, latency_ms: 320, status: 'ok' },
          { model: 'm2', ok: false, latency_ms: 0, status: 'timeout' },
        ]),
      )
      const w = await toAiPage()
      await w.find('[data-service=jev] input[type=url]').setValue('https://jev.example')
      await w.find('[data-service=llm] input[type=url]').setValue('https://llm.example')
      await w.find('.test button').trigger('click')
      await flushPromises()
      expect(mocks.secretsSet).toHaveBeenCalledTimes(1)
      const items = w.findAll('.results li').map((li) => li.text())
      expect(items).toEqual([
        'm1：可以用，320 毫秒',
        'm2：连不上（timeout）',
        '情绪识别没有测试接口，用起来以后看小组件上有没有“离线”角标',
      ])
    })

    it('保存失败：说明原因，停在这一页', async () => {
      mocks.secretsSet.mockImplementation(() =>
        Promise.resolve({
          status: 'error',
          error: { code: 'ai.config_invalid', message_key: 'error.ai_config_invalid' },
        }),
      )
      const w = await toAiPage()
      await w.find('[data-service=llm] input[type=url]').setValue('ftp://x')
      await w.find('button.primary').trigger('click')
      await flushPromises()
      expect(w.find('[role=alert]').text()).toContain('地址要以 http:// 或 https:// 开头')
      expect(w.find('[data-service=llm]').exists()).toBe(true)
    })

    it('开发版连着 mock-ai 时给一行提示', async () => {
      mocks.aiConfigGet.mockImplementation(() => ok({ source: 'dev_mock', jev: null, llm: null }))
      const w = await toAiPage()
      expect(w.find('.dev').text()).toBe('开发版：正在连本机的 mock-ai')
    })
  })

  describe('偏好（FR-ONB-05 第 2 条）', () => {
    async function toPrefsPage() {
      const w = await toAiPage()
      await w.find('button.skip').trigger('click')
      await flushPromises()
      return w
    }

    it('关怀频率与休息提醒显示当前值，改了就保存', async () => {
      const w = await toPrefsPage()
      const care = w.findAll<HTMLInputElement>('[data-pref=care] input')
      expect(care.filter((r) => r.element.checked).map((r) => r.element.value)).toEqual(['normal'])
      const rest = w.findAll<HTMLInputElement>('[data-pref=rest] input')
      expect(rest.map((r) => r.element.checked)).toEqual([true, true, true, false])
      await care[2]!.setValue(true)
      expect(mocks.settingsSet).toHaveBeenLastCalledWith('care.level', 'less')
      await rest[3]!.setValue(true)
      expect(mocks.settingsSet).toHaveBeenLastCalledWith('rest.night.enabled', true)
    })

    it('选小组件的角：存在本机，并忘掉之前记住的位置', async () => {
      localStorage.setItem('xq.widget.placement', '{"last":"a","byScreen":{}}')
      const w = await toPrefsPage()
      const corners = w.findAll<HTMLInputElement>('[data-pref=corner] input')
      expect(corners[0]!.element.checked).toBe(true) // 默认右下角
      await corners[3]!.setValue(true)
      expect(localStorage.getItem('xq.widget.corner')).toBe('tl')
      expect(localStorage.getItem('xq.widget.placement')).toBeNull()
    })
  })

  it('未满 18 周岁：说明后只能关闭，不进入同意页（FR-ONB-02）', async () => {
    const w = mountApp()
    await flushPromises()
    await w.findAll('button')[1]!.trigger('click')
    expect(w.text()).toContain('心晴的陪伴功能暂时只面向成年人')
    expect(w.find('input[type=checkbox]').exists()).toBe(false)
    await w.find('button').trigger('click')
    expect(mocks.close).toHaveBeenCalled()
  })
})
