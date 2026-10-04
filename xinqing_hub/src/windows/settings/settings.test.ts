import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const mocks = vi.hoisted(() => ({ getVersion: vi.fn() }))

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    settingsGet: async (key: string) => ({ status: 'ok', data: key === 'ui.theme' ? 'system' : 1 }),
    settingsSet: async () => ({ status: 'ok', data: null }),
  },
  events: { settingsChanged: { listen: async () => () => {} } },
}))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: mocks.getVersion }))

const { default: App } = await import('./App.vue')

async function openAbout() {
  const w = mount(App)
  await flushPromises()
  const nav = w.findAll('nav button')
  await nav.find((b) => b.text() === '隐私与关于')!.trigger('click')
  await flushPromises()
  return w
}

describe('设置中心', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    location.hash = ''
    mocks.getVersion.mockReset().mockResolvedValue('0.1.0')
  })

  it('左侧分类按 FR-SET-01 命名，当前分类有 aria-current', async () => {
    const w = mount(App)
    await flushPromises()
    const nav = w.findAll('nav button')
    expect(nav.map((b) => b.text())).toEqual(['外观', '隐私与关于'])
    expect(nav[0]!.attributes('aria-current')).toBe('page')
    expect(w.find('h1').text()).toBe('外观')
  })

  it('“关于”写明非官方分支、免责声明和版本号（FR-SET-09、C-LAW-08）', async () => {
    const w = await openAbout()
    expect(w.find('h1').text()).toBe('隐私与关于')
    const text = w.text()
    expect(text).toContain('心晴基于开源项目清风输入法（WindInput，MIT 协议）开发，为非官方版本。')
    expect(text).toContain('本产品不提供任何医疗诊断或治疗。')
    expect(w.find('.version').text()).toBe('版本 0.1.0')
    expect(w.find('[role=img]').attributes('aria-label')).toBe('心晴')
  })

  it('开源许可列表：清风 MIT 全文与第三方资源声明都随包带着', async () => {
    const w = await openAbout()
    const full = w.findAll('details')
    expect(full).toHaveLength(2)
    expect(full[0]!.find('summary').text()).toContain('MIT')
    expect(full[0]!.find('pre').text()).toContain('Permission is hereby granted')
    expect(full[1]!.find('pre').text()).toContain('第三方资源声明')
    expect(w.findAll('.licenses li').map((li) => li.find('.lib').text())).toEqual(
      expect.arrayContaining(['Tauri', 'Vue', 'Pinia']),
    )
  })

  it('拿不到版本号（浏览器预览）时不显示那一行', async () => {
    mocks.getVersion.mockRejectedValue(new Error('no tauri'))
    const w = await openAbout()
    expect(w.find('.version').exists()).toBe(false)
  })

  it('地址带 #about 时直接打开“隐私与关于”', async () => {
    location.hash = '#about'
    const w = mount(App)
    await flushPromises()
    expect(w.find('h1').text()).toBe('隐私与关于')
  })
})
