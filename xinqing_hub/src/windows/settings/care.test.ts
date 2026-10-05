import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { MemoryItem, SettingValue } from '@/api'
import { t } from '@/i18n'
import { crossesMidnight, hasApp, quietRange, showRange, validApp, validMemory, validPhone } from './care'

describe('关怀设置的输入校验（与后端 SettingItem::accepts 一致）', () => {
  it('进程名：不带路径、不超过 64 字、首尾没有空白', () => {
    expect(validApp('Zoom.exe')).toBe(true)
    expect(validApp('腾讯会议.exe')).toBe(true)
    for (const bad of ['', ' Zoom.exe', 'C:\\Zoom.exe', 'a/b.exe', 'a:b', 'x\ty', 'a'.repeat(65)])
      expect(validApp(bad), bad).toBe(false)
    expect(validApp('a'.repeat(64))).toBe(true)
  })

  it('勿扰应用去重忽略大小写', () => {
    expect(hasApp(['Zoom.exe'], 'zoom.EXE')).toBe(true)
    expect(hasApp(['Zoom.exe'], 'Teams.exe')).toBe(false)
  })

  it('电话：空串表示没填；数字、空格与 +-()，至少 3 个数字，不超过 24 字', () => {
    for (const good of ['', '110', '0571-8888 1234', '+86 (571) 8888-1234'])
      expect(validPhone(good), good).toBe(true)
    for (const bad of ['12', 'abc123', ' 110', '110 ', '1'.repeat(25), '电话110'])
      expect(validPhone(bad), bad).toBe(false)
  })

  it('安静时段：两个时间拼成 HH:MM-HH:MM，起止相同不收；跨午夜的能认出来', () => {
    expect(quietRange('22:00', '07:00')).toBe('22:00-07:00')
    expect(quietRange('12:00', '12:00')).toBeNull()
    expect(quietRange('', '07:00')).toBeNull()
    expect(showRange('22:00-07:00')).toBe('22:00–07:00')
    expect(crossesMidnight('22:00-07:00')).toBe(true)
    expect(crossesMidnight('12:00-13:30')).toBe(false)
  })

  it('要记住的事：去掉首尾空白后非空，不超过 100 字', () => {
    expect(validMemory(' 下周三考试 ')).toBe(true)
    expect(validMemory('   ')).toBe(false)
    expect(validMemory('字'.repeat(100))).toBe(true)
    expect(validMemory('字'.repeat(101))).toBe(false)
  })
})

const mocks = vi.hoisted(() => ({
  values: {} as Record<string, SettingValue>,
  settingsSet: vi.fn(),
  consentGet: vi.fn(),
  consentSet: vi.fn(),
  memories: [] as MemoryItem[],
  memoryUpdate: vi.fn(),
  memoryDelete: vi.fn(),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })
const consent = (granted: boolean) => ({ policy_ver: 1, items: [{ item: 'llm_summary', granted }] })

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    settingsGet: async (key: string) => ok(mocks.values[key]),
    settingsSet: mocks.settingsSet,
    settingsSchema: async () => [
      { key: 'care.quiet_hours', kind: { type: 'list', item: 'time_range', max_items: 2 }, default: [] },
    ],
    consentGet: mocks.consentGet,
    consentSet: mocks.consentSet,
    memoryList: async () => ok(mocks.memories.map((m) => ({ ...m }))),
    memoryUpdate: mocks.memoryUpdate,
    memoryDelete: mocks.memoryDelete,
  },
  events: { settingsChanged: { listen: async () => () => {} } },
}))

const { default: CareSection } = await import('./CareSection.vue')

enableAutoUnmount(afterEach)

async function mountCare(): Promise<VueWrapper> {
  const w = mount(CareSection, { attachTo: document.body })
  await flushPromises()
  return w
}
const group = (w: VueWrapper, g: string) => w.find(`[data-group=${g}]`)

describe('设置中心 · 关怀（FR-SET-05）', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    mocks.values = {
      'care.level': 'normal',
      'care.style': 'gentle',
      'care.quiet_hours': ['22:00-07:00'],
      'care.dnd_apps': ['wemeetapp.exe', 'Zoom.exe'],
      'safety.school_phone': '',
    }
    mocks.settingsSet.mockReset().mockImplementation(() => ok())
    mocks.consentGet.mockReset().mockImplementation(() => ok(consent(true)))
    mocks.consentSet.mockReset().mockImplementation((_: string, g: boolean) => ok(consent(g)))
    mocks.memories = [
      { id: 2, content: '下周三有期中考试', created_ts: Date.UTC(2026, 9, 3) },
      { id: 1, content: '喜欢猫', created_ts: Date.UTC(2026, 9, 1) },
    ]
    mocks.memoryUpdate.mockReset().mockImplementation(() => ok())
    mocks.memoryDelete.mockReset().mockImplementation(() => ok())
  })

  it('关怀频率与说话风格：回填当前值，点了就存', async () => {
    const w = await mountCare()
    const levels = group(w, 'level').findAll('input[type=radio]')
    expect(levels.map((r) => (r.element as HTMLInputElement).checked)).toEqual([false, true, false, false])
    expect(group(w, 'level').text()).toContain(t('care_level.off'))
    await levels[2]!.setValue(true)
    expect(mocks.settingsSet).toHaveBeenCalledWith('care.level', 'less')

    const style = group(w, 'style')
    expect(style.text()).toContain(t('settings.care.style_lively_eg'))
    await style.findAll('input[type=radio]')[2]!.setValue(true)
    expect(mocks.settingsSet).toHaveBeenCalledWith('care.style', 'brief')
  })

  it('撤回同意 ④：调 consent_set，并如实说明已发出的收不回来', async () => {
    const w = await mountCare()
    const box = group(w, 'summary').find('input[type=checkbox]')
    expect((box.element as HTMLInputElement).checked).toBe(true)
    expect(group(w, 'summary').find('[role=status]').exists()).toBe(false)
    await box.setValue(false)
    await flushPromises()
    expect(mocks.consentSet).toHaveBeenCalledWith('llm_summary', false)
    expect(group(w, 'summary').find('[role=status]').text()).toBe(t('error.withdraw_notice'))
  })

  it('安静时段：跨午夜的标“次日”；添加、删除都提交整份列表；起止相同先拦下；到上限就不再给添加', async () => {
    const w = await mountCare()
    const g = group(w, 'quiet')
    expect(g.find('.items').text()).toContain('22:00–07:00')
    expect(g.find('.items').text()).toContain(t('settings.care.quiet_next_day'))

    const [from, to] = g.findAll('input[type=time]')
    await from!.setValue('12:00')
    await to!.setValue('12:00')
    await g.find('form').trigger('submit')
    expect(g.find('[role=alert]').text()).toBe(t('settings.care.quiet_invalid'))
    expect(mocks.settingsSet).not.toHaveBeenCalled()

    await to!.setValue('13:30')
    await g.find('form').trigger('submit')
    await flushPromises()
    expect(mocks.settingsSet).toHaveBeenLastCalledWith('care.quiet_hours', ['22:00-07:00', '12:00-13:30'])
    // schema 说最多 2 段：满了就不显示添加
    expect(group(w, 'quiet').find('form').exists()).toBe(false)

    await group(w, 'quiet').find('button[aria-label="删掉 22:00–07:00"]').trigger('click')
    await flushPromises()
    expect(mocks.settingsSet).toHaveBeenLastCalledWith('care.quiet_hours', ['12:00-13:30'])
  })

  it('勿扰应用：带路径、重复的先拦下；合格的追加到列表末尾', async () => {
    const w = await mountCare()
    const g = group(w, 'dnd')
    const input = g.find('input[type=text]')
    await input.setValue('C:\\Program Files\\x.exe')
    await g.find('form').trigger('submit')
    expect(g.find('[role=alert]').text()).toBe(t('settings.care.dnd_invalid'))
    await input.setValue('zoom.EXE')
    await g.find('form').trigger('submit')
    expect(g.find('[role=alert]').text()).toBe(t('settings.care.dnd_duplicate'))
    expect(mocks.settingsSet).not.toHaveBeenCalled()

    await input.setValue('  ms-teams.exe ')
    await g.find('form').trigger('submit')
    await flushPromises()
    expect(mocks.settingsSet).toHaveBeenCalledWith('care.dnd_apps', [
      'wemeetapp.exe',
      'Zoom.exe',
      'ms-teams.exe',
    ])
    expect((input.element as HTMLInputElement).value).toBe('')
    expect(group(w, 'dnd').find('[role=alert]').exists()).toBe(false)
  })

  it('学校心理中心电话：格式不对先提示，不提交；合格的去掉首尾空白再存', async () => {
    const w = await mountCare()
    const input = group(w, 'phone').find('input[type=tel]')
    await input.setValue('打给我')
    expect(group(w, 'phone').find('[role=alert]').text()).toBe(t('settings.care.phone_invalid'))
    expect(mocks.settingsSet).not.toHaveBeenCalled()
    await input.setValue(' 0571-8888 1234 ')
    await flushPromises()
    expect(mocks.settingsSet).toHaveBeenCalledWith('safety.school_phone', '0571-8888 1234')
    expect(group(w, 'phone').find('[role=alert]').exists()).toBe(false)
  })

  it('晴晴记住的事：列出、改一下（Esc 取消、超长先拦）、删除要再点一次确认', async () => {
    const w = await mountCare()
    const g = () => group(w, 'memory')
    expect(g().find('legend').text()).toContain('2 / 50')
    const items = () => g().findAll('.item')
    expect(items().map((i) => i.find('.name').text())).toEqual([
      expect.stringContaining('下周三有期中考试'),
      expect.stringContaining('喜欢猫'),
    ])

    await items()[1]!.findAll('button')[0]!.trigger('click')
    await flushPromises()
    const edit = items()[1]!.find('input')
    expect(document.activeElement).toBe(edit.element)
    await edit.trigger('keydown', { key: 'Escape' })
    expect(items()[1]!.find('input').exists()).toBe(false)

    await items()[1]!.findAll('button')[0]!.trigger('click')
    await flushPromises()
    await items()[1]!.find('input').setValue('字'.repeat(101))
    await items()[1]!.find('input').trigger('keydown', { key: 'Enter' })
    expect(g().find('[role=alert]').text()).toBe(t('error.memory_invalid'))
    expect(mocks.memoryUpdate).not.toHaveBeenCalled()
    await items()[1]!.find('input').setValue(' 喜欢猫，尤其是橘猫 ')
    await items()[1]!.find('input').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(mocks.memoryUpdate).toHaveBeenCalledWith(1, '喜欢猫，尤其是橘猫')
    expect(items()[1]!.find('.name').text()).toContain('喜欢猫，尤其是橘猫')

    const del = () => items()[0]!.findAll('button')[1]!
    await del().trigger('click')
    expect(mocks.memoryDelete).not.toHaveBeenCalled()
    expect(del().text()).toBe(t('settings.care.memory_delete_confirm'))
    await del().trigger('click')
    await flushPromises()
    expect(mocks.memoryDelete).toHaveBeenCalledWith(2)
    expect(items()).toHaveLength(1)
    expect(g().find('legend').text()).toContain('1 / 50')
  })

  it('没有记住的事时给出怎么让晴晴记住的说明；回到窗口时重读一次', async () => {
    mocks.memories = []
    const w = await mountCare()
    expect(group(w, 'memory').text()).toContain(t('settings.care.memory_empty'))
    mocks.memories = [{ id: 3, content: '周五交论文', created_ts: Date.UTC(2026, 9, 5) }]
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(group(w, 'memory').find('.item').text()).toContain('周五交论文')
  })
})
