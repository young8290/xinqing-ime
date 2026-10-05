import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

const mocks = vi.hoisted(() => ({
  getVersion: vi.fn(),
  imeSchema: vi.fn(),
  imeConfigGet: vi.fn(),
  imeConfigSet: vi.fn(),
  /** 最近一次订阅 `ime_config:changed` 的回调 */
  imeChanged: undefined as ((e: { payload: unknown }) => void) | undefined,
  imeUnlisten: vi.fn(),
  aiConfigGet: vi.fn(),
  secretsSet: vi.fn(),
  aiTestConnection: vi.fn(),
  aiUsageToday: vi.fn(),
  settingsSet: vi.fn(),
}))
const ok = (data: unknown) => Promise.resolve({ status: 'ok', data })
const CAPS: Record<string, number> = {
  'ai.cap.jev': 3000,
  'ai.cap.llm': 200,
  'ai.cap.chat': 100,
  'ai.cap.schedule': 50,
  'ai.cap.rewrite': 100,
}
const AI_VIEW = {
  source: 'saved',
  jev: { base_url: 'https://jev.example', key_tail: '••••a1b2', model: 'jev-latest' },
  llm: { base_url: 'https://llm.example', key_tail: '••••c3d4', models: ['m1', 'm2', 'm3'] },
}
const SCHEMA = [
  { key: 'schema.active', kind: 'string', options: null },
  { key: 'schema.available', kind: 'string_list', options: null },
  { key: 'ui.candidate.per_page', kind: 'int', options: null },
  { key: 'ui.candidate.layout', kind: 'enum', options: ['horizontal', 'vertical'] },
  { key: 'keys.toggle_mode_keys', kind: 'string_list', options: null },
  { key: 'keys.switch_engine', kind: 'string', options: null },
  { key: 'keys.toggle_punct', kind: 'string', options: null },
  { key: 'keys.pin_candidate', kind: 'string', options: null },
  { key: 'input.emoji.enabled', kind: 'bool', options: null },
  { key: 'ui.font.scripts', kind: 'map', options: null },
]
const CONFIG = {
  schema: { active: 'pinyin', available: ['pinyin', 'wubi86'] },
  ui: { candidate: { per_page: 7, layout: 'horizontal' }, font: { scripts: { han: 'YaHei' } } },
  keys: {
    toggle_mode_keys: ['lshift'],
    switch_engine: 'ctrl+shift+e',
    toggle_punct: 'ctrl+.',
    pin_candidate: 'ctrl+number',
  },
  input: { emoji: { enabled: false } },
}

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    settingsGet: async (key: string) => ({
      status: 'ok',
      data: key === 'ui.theme' ? 'system' : key.startsWith('ai.cap.') ? CAPS[key] : 1,
    }),
    settingsSet: mocks.settingsSet,
    settingsSchema: async () => [
      { key: 'ai.cap.llm', kind: { type: 'int', min: 0, max: 2000 }, default: 200 },
      { key: 'ui.theme', kind: { type: 'choice', options: ['system'] }, default: 'system' },
    ],
    aiConfigGet: mocks.aiConfigGet,
    secretsSet: mocks.secretsSet,
    aiTestConnection: mocks.aiTestConnection,
    aiUsageToday: mocks.aiUsageToday,
    consentGet: async () => ({ status: 'ok', data: { policy_ver: 1, items: [] } }),
    aiNetLogRecent: async () => ({ status: 'ok', data: [] }),
    imeSchema: mocks.imeSchema,
    imeConfigGet: mocks.imeConfigGet,
    imeConfigSet: mocks.imeConfigSet,
  },
  events: {
    settingsChanged: { listen: async () => () => {} },
    imeConfigChanged: {
      listen: async (cb: (e: { payload: unknown }) => void) => {
        mocks.imeChanged = cb
        return mocks.imeUnlisten
      },
    },
  },
}))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: mocks.getVersion }))

const { default: App } = await import('./App.vue')

// 每个用例结束卸载窗口：输入法页挂着窗口 focus 监听，留着会串到后面的用例
enableAutoUnmount(afterEach)

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
    mocks.imeSchema.mockReset().mockImplementation(() => ok(SCHEMA))
    mocks.imeConfigGet.mockReset().mockImplementation(() => ok({ values: CONFIG }))
    mocks.imeConfigSet
      .mockReset()
      .mockImplementation(() => ok({ needs_restart: false, applied: 1, skipped: [] }))
    mocks.imeChanged = undefined
    mocks.imeUnlisten.mockReset()
    mocks.aiConfigGet.mockReset().mockImplementation(() => ok(AI_VIEW))
    mocks.secretsSet.mockReset().mockImplementation(() => ok(AI_VIEW))
    mocks.aiTestConnection.mockReset().mockImplementation(() => ok([]))
    mocks.aiUsageToday.mockReset().mockImplementation(() => ok([{ kind: 'llm', used: 12, cap: 200 }]))
    mocks.settingsSet.mockReset().mockImplementation(() => ok(null))
  })

  it('左侧分类按 FR-SET-01 命名，当前分类有 aria-current', async () => {
    const w = mount(App)
    await flushPromises()
    const nav = w.findAll('nav button')
    expect(nav.map((b) => b.text())).toEqual(['输入法', '外观', '关怀', 'AI 服务', '隐私与关于'])
    expect(nav[0]!.attributes('aria-current')).toBe('page')
    expect(w.find('h1').text()).toBe('输入法')
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

  describe('输入法（FR-SET-02、ADR 0016）', () => {
    async function openIme() {
      const w = mount(App, { attachTo: document.body })
      await flushPromises()
      return w
    }
    const row = (w: Awaited<ReturnType<typeof openIme>>, key: string) => w.find(`[data-key="${key}"]`)
    async function openGroup(w: Awaited<ReturnType<typeof openIme>>, prefix: string) {
      const g = w.findAll('details.group').find((d) => d.find('.prefix').text() === prefix)!
      ;(g.element as HTMLDetailsElement).open = true
      g.element.dispatchEvent(new Event('toggle'))
      await flushPromises()
    }

    it('默认打开输入法；常用项有中文名称，输入方案用 schema.available 做下拉、显示方案名', async () => {
      const w = await openIme()
      const common = w
        .findAll('.row')
        .slice(0, 4)
        .map((r) => r.find('label').text())
      expect(common).toEqual(['输入方案', '候选个数', '候选排列', '中英切换键'])
      const opts = row(w, 'schema.active')
        .findAll('option')
        .map((o) => o.text())
      expect(opts).toEqual(['全拼', '五笔 86'])
      expect((row(w, 'ui.candidate.per_page').find('input').element as HTMLInputElement).value).toBe('7')
      w.unmount()
    })

    it('修改即保存：提交这一项、显示“已保存”、再取一次配置', async () => {
      const w = await openIme()
      const input = row(w, 'ui.candidate.per_page').find('input')
      await input.setValue('9')
      await flushPromises()
      expect(mocks.imeConfigSet).toHaveBeenCalledTimes(1)
      expect(mocks.imeConfigSet).toHaveBeenCalledWith([{ key: 'ui.candidate.per_page', value: 9 }])
      expect(w.find('.saved').text()).toBe('已保存')
      expect(mocks.imeConfigGet).toHaveBeenCalledTimes(2)
      w.unmount()
    })

    it('核心跳过的项把原因显示在控件下方；需要重启时显示横幅', async () => {
      mocks.imeConfigSet.mockImplementation(() =>
        ok({
          needs_restart: true,
          applied: 0,
          skipped: [{ key: 'ui.candidate.layout', reason: '取值不对' }],
        }),
      )
      const w = await openIme()
      await row(w, 'ui.candidate.layout').find('select').setValue('vertical')
      await flushPromises()
      expect(row(w, 'ui.candidate.layout').find('[role=alert]').text()).toBe('取值不对')
      expect(w.find('.banner').text()).toBe('部分设置需要重启输入法后生效')
      expect(w.find('.saved').exists()).toBe(false)
      w.unmount()
    })

    it('中英切换键只能从五个单键里选，显示中文名，整份提交', async () => {
      const w = await openIme()
      const r = row(w, 'keys.toggle_mode_keys')
      expect(r.find('.chip').text()).toBe('左 Shift')
      const choices = r.findAll('select option:not([disabled])').map((o) => o.text())
      expect(choices).toEqual(['右 Shift', '左 Ctrl', '右 Ctrl', 'Caps Lock'])
      await r.find('select').setValue('rshift')
      expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([
        { key: 'keys.toggle_mode_keys', value: ['lshift', 'rshift'] },
      ])
      await r.find('button.remove').trigger('click')
      expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([{ key: 'keys.toggle_mode_keys', value: [] }])
      w.unmount()
    })

    it('其他字符串列表手动输入添加', async () => {
      const w = await openIme()
      await openGroup(w, 'schema')
      const r = row(w, 'schema.available')
      await r.find('input.add').setValue('stroke')
      await r.find('input.add').trigger('keydown', { key: 'Enter' })
      expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([
        { key: 'schema.available', value: ['pinyin', 'wubi86', 'stroke'] },
      ])
      w.unmount()
    })

    it('高级区按键名第一段分组，展开才渲染；map 类型只读显示 JSON', async () => {
      const w = await openIme()
      const groups = w.findAll('details.group')
      expect(groups.map((g) => g.find('.prefix').text())).toEqual(['input', 'keys', 'schema', 'ui'])
      expect(row(w, 'input.emoji.enabled').exists()).toBe(false)
      const input = groups[0]!.element as HTMLDetailsElement
      input.open = true
      input.dispatchEvent(new Event('toggle'))
      await flushPromises()
      await row(w, 'input.emoji.enabled').find('input[type=checkbox]').setValue(true)
      expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([{ key: 'input.emoji.enabled', value: true }])
      const ui = groups[3]!.element as HTMLDetailsElement
      ui.open = true
      ui.dispatchEvent(new Event('toggle'))
      await flushPromises()
      expect(row(w, 'ui.font.scripts').find('pre').text()).toContain('YaHei')
      expect(row(w, 'ui.font.scripts').text()).toContain('只能查看')
      w.unmount()
    })

    it('收到 ime_config:changed 重新取配置；别处改了需要重启的项也显示横幅', async () => {
      const w = await openIme()
      mocks.imeConfigGet.mockImplementation(() =>
        ok({ values: { ...CONFIG, ui: { ...CONFIG.ui, candidate: { per_page: 5, layout: 'vertical' } } } }),
      )
      mocks.imeChanged!({ payload: { reason: 'setItems', needs_restart: true } })
      await flushPromises()
      expect(mocks.imeConfigGet).toHaveBeenCalledTimes(2)
      expect(mocks.imeSchema).toHaveBeenCalledTimes(1)
      expect((row(w, 'ui.candidate.per_page').find('input').element as HTMLInputElement).value).toBe('5')
      expect(w.find('.banner').exists()).toBe(true)
      w.unmount()
    })

    it('窗口重新获得焦点时重新取配置（核心的语言栏、菜单改配置不广播）', async () => {
      const w = await openIme()
      window.dispatchEvent(new Event('focus'))
      await flushPromises()
      expect(mocks.imeConfigGet).toHaveBeenCalledTimes(2)
      w.unmount()
    })

    it('取配置的过程中连来几次信号，结束后只补取一次', async () => {
      const w = await openIme()
      let release!: () => void
      mocks.imeConfigGet.mockImplementationOnce(
        () => new Promise((r) => (release = () => r({ status: 'ok', data: { values: CONFIG } }))),
      )
      const change = { payload: { reason: 'setItems', needs_restart: false } }
      mocks.imeChanged!(change)
      mocks.imeChanged!(change)
      window.dispatchEvent(new Event('focus'))
      release()
      await flushPromises()
      // 打开时 1 次 + 第一次信号 1 次 + 合并后补取 1 次
      expect(mocks.imeConfigGet).toHaveBeenCalledTimes(3)
      w.unmount()
    })

    it('核心没运行时停在说明页；核心启动后（connected）自动加载，不用点重试', async () => {
      mocks.imeSchema.mockImplementation(() =>
        Promise.resolve({
          status: 'error',
          error: { code: 'ime.unavailable', message_key: 'error.ime_unavailable' },
        }),
      )
      const w = await openIme()
      expect(w.find('[role=status]').exists()).toBe(true)
      mocks.imeSchema.mockImplementation(() => ok(SCHEMA))
      mocks.imeChanged!({ payload: { reason: 'connected', needs_restart: false } })
      await flushPromises()
      expect(row(w, 'schema.active').exists()).toBe(true)
      expect(w.find('.banner').exists()).toBe(false)
      w.unmount()
    })

    it('离开页面时退订事件、不再响应焦点', async () => {
      const w = await openIme()
      w.unmount()
      expect(mocks.imeUnlisten).toHaveBeenCalledTimes(1)
      window.dispatchEvent(new Event('focus'))
      await flushPromises()
      expect(mocks.imeConfigGet).toHaveBeenCalledTimes(1)
    })

    describe('快捷键录制框（ADR 0016 第 5 条）', () => {
      async function recorder() {
        const w = await openIme()
        await openGroup(w, 'keys')
        const r = row(w, 'keys.switch_engine')
        return { w, r, button: r.find('button.record') }
      }
      const press = (code: string, mods: Record<string, boolean> = {}) => ({
        code,
        ctrlKey: !!mods.ctrl,
        altKey: !!mods.alt,
        shiftKey: !!mods.shift,
        metaKey: false,
      })

      it('快捷键有中文名称、按 Ctrl + Shift + E 的样子显示；点一下录制，按下组合键即保存', async () => {
        const { r, button } = await recorder()
        expect(r.find('label').text()).toBe('轮换输入方案')
        expect(button.text()).toBe('Ctrl + Shift + E')
        expect(button.attributes('aria-label')).toBe('修改“轮换输入方案”，现在是 Ctrl + Shift + E')
        await button.trigger('click')
        expect(button.attributes('aria-pressed')).toBe('true')
        await button.trigger('keydown', press('ControlLeft', { ctrl: true }))
        expect(button.text()).toBe('Ctrl + …')
        await button.trigger('keydown', press('KeyK', { ctrl: true, alt: true }))
        expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([
          { key: 'keys.switch_engine', value: 'ctrl+alt+k' },
        ])
        expect(button.attributes('aria-pressed')).toBe('false')
      })

      it('和别的快捷键重复时提示、不保存（含 Ctrl+数字 候选模板）', async () => {
        const { r, button } = await recorder()
        await button.trigger('click')
        await button.trigger('keydown', press('Period', { ctrl: true }))
        expect(r.find('[role=alert]').text()).toBe('已经给了“中文 / 英文标点”，先把那边改掉吧')
        await button.trigger('click')
        await button.trigger('keydown', press('Digit3', { ctrl: true }))
        expect(r.find('[role=alert]').text()).toContain('固定候选')
        expect(mocks.imeConfigSet).not.toHaveBeenCalled()
      })

      it('会和打字冲突的不收；Esc 取消；单按退格、点“清除”都写入 none', async () => {
        const { r, button } = await recorder()
        await button.trigger('click')
        await button.trigger('keydown', press('KeyK'))
        expect(r.find('[role=alert]').text()).toBe('要带上 Ctrl、Alt 或 Win，免得和打字冲突')
        await button.trigger('keydown', press('Escape'))
        expect(button.attributes('aria-pressed')).toBe('false')
        expect(mocks.imeConfigSet).not.toHaveBeenCalled()
        await button.trigger('click')
        await button.trigger('keydown', press('Backspace'))
        expect(mocks.imeConfigSet).toHaveBeenLastCalledWith([{ key: 'keys.switch_engine', value: 'none' }])
        await r.find('button.clear').trigger('click')
        expect(mocks.imeConfigSet).toHaveBeenCalledTimes(2)
      })

      it('录制中失去焦点就取消', async () => {
        const { button } = await recorder()
        await button.trigger('click')
        await button.trigger('blur')
        expect(button.attributes('aria-pressed')).toBe('false')
        expect(button.text()).toBe('Ctrl + Shift + E')
      })
    })

    it('输入法核心没运行：说明原因并给“重试”', async () => {
      mocks.imeSchema.mockImplementation(() =>
        ok(null).then(() => ({
          status: 'error',
          error: { code: 'ime.unavailable', message_key: 'error.ime_unavailable' },
        })),
      )
      const w = await openIme()
      expect(w.find('[role=status]').text()).toContain('输入法还没启动')
      mocks.imeSchema.mockImplementation(() => ok(SCHEMA))
      await w.find('[role=status] button').trigger('click')
      await flushPromises()
      expect(row(w, 'schema.active').exists()).toBe(true)
      w.unmount()
    })
  })

  describe('AI 服务（FR-SET-08）', () => {
    async function openAi() {
      const w = mount(App, { attachTo: document.body })
      await flushPromises()
      await w
        .findAll('nav button')
        .find((b) => b.text() === 'AI 服务')!
        .trigger('click')
      await flushPromises()
      return w
    }
    const models = (w: Awaited<ReturnType<typeof openAi>>) => w.findAll('.model .name').map((n) => n.text())

    it('地址填好，密钥只显示末 4 位；没改动时“保存”不可点', async () => {
      const w = await openAi()
      expect(w.find('h1').text()).toBe('AI 服务')
      const llm = w.find('[data-service=llm]')
      expect((llm.find('input[type=url]').element as HTMLInputElement).value).toBe('https://llm.example')
      expect(llm.find('input[type=password]').attributes('placeholder')).toBe('已保存 ••••c3d4，留空就不改')
      expect(models(w)).toEqual(['m1', 'm2', 'm3'])
      expect(w.find('.row-actions button.primary').attributes('disabled')).toBeDefined()
    })

    it('调整大模型顺序、增删模型后保存，按新顺序提交', async () => {
      const w = await openAi()
      await w.findAll('.model')[0]!.findAll('button')[1]!.trigger('click') // m1 往后挪
      expect(models(w)).toEqual(['m2', 'm1', 'm3'])
      await w.findAll('.model')[2]!.findAll('button')[2]!.trigger('click') // 删掉 m3
      await w.find('.add input').setValue('m9')
      await w.find('.add input').trigger('keydown', { key: 'Enter' })
      expect(models(w)).toEqual(['m2', 'm1', 'm9'])
      await w.find('.row-actions button.primary').trigger('click')
      await flushPromises()
      expect(mocks.secretsSet).toHaveBeenCalledWith({
        jev: { base_url: 'https://jev.example', api_key: null, model: 'jev-latest' },
        llm: { base_url: 'https://llm.example', api_key: null, models: ['m2', 'm1', 'm9'] },
      })
      expect(w.find('.row-actions [role=status]').text()).toBe('已保存')
    })

    it('拖动排序：把第三个拖到最前', async () => {
      const w = await openAi()
      const items = w.findAll('.model')
      await items[2]!.trigger('dragstart')
      await items[0]!.trigger('drop')
      expect(models(w)).toEqual(['m3', 'm1', 'm2'])
    })

    it('测试连接：没改动时不保存，逐个模型显示结果', async () => {
      mocks.aiTestConnection.mockImplementation(() =>
        ok([
          { model: 'm1', ok: true, latency_ms: 300, status: 'ok' },
          { model: 'm2', ok: false, latency_ms: 0, status: '401' },
        ]),
      )
      const w = await openAi()
      await w.findAll('.row-actions button')[1]!.trigger('click')
      await flushPromises()
      expect(mocks.secretsSet).not.toHaveBeenCalled()
      expect(w.findAll('.results li').map((li) => li.text())).toEqual([
        'm1：可以用，300 毫秒',
        'm2：连不上（401）',
        '情绪识别没有测试接口，用起来以后看小组件上有没有“离线”角标',
      ])
    })

    it('每日上限显示今日用量；改了就存并刷新用量，范围取自 settings_schema', async () => {
      const w = await openAi()
      const llm = w.find('[data-kind=llm]')
      expect(llm.find('.usage').text()).toBe('今天已用 12 / 200')
      expect(llm.find('input').attributes('max')).toBe('2000')
      await llm.find('input').setValue('150')
      await flushPromises()
      expect(mocks.settingsSet).toHaveBeenCalledWith('ai.cap.llm', 150)
      expect(mocks.aiUsageToday).toHaveBeenCalledTimes(2)
    })

    it('没有配置时说明是离线模式', async () => {
      mocks.aiConfigGet.mockImplementation(() => ok({ source: 'none', jev: null, llm: null }))
      const w = await openAi()
      expect(w.find('.note').text()).toBe('现在是离线模式：没填 AI 服务，只有本机的功能。')
    })
  })
})
