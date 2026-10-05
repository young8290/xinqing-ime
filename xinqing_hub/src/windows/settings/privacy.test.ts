import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type { ConsentItem, NetLogView } from '@/api'
import { t } from '@/i18n'

const mocks = vi.hoisted(() => ({
  consentGet: vi.fn(),
  consentSet: vi.fn(),
  netlog: [] as NetLogView[],
  dataExport: vi.fn(),
  save: vi.fn(),
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })
const ITEMS: ConsentItem[] = ['sense', 'jev_features', 'schedule', 'llm_summary', 'enhanced', 'rewrite']
const state = (granted: Partial<Record<ConsentItem, boolean>>) => ({
  policy_ver: 1,
  items: ITEMS.map((item) => ({ item, granted: granted[item] ?? false })),
})

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    consentGet: mocks.consentGet,
    consentSet: mocks.consentSet,
    aiNetLogRecent: async () => ok(mocks.netlog),
    dataExport: mocks.dataExport,
  },
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save }))

const { default: PrivacySection } = await import('./PrivacySection.vue')

enableAutoUnmount(afterEach)

async function mountPrivacy(): Promise<VueWrapper> {
  const w = mount(PrivacySection)
  await flushPromises()
  return w
}
const box = (w: VueWrapper, item: ConsentItem) => w.find(`[data-item=${item}] input`)

describe('设置中心 · 隐私（FR-SET-09）', () => {
  let current: Partial<Record<ConsentItem, boolean>>

  beforeEach(() => {
    current = { sense: true, jev_features: true, llm_summary: true }
    mocks.consentGet.mockReset().mockImplementation(() => ok(state(current)))
    mocks.consentSet.mockReset().mockImplementation((item: ConsentItem, granted: boolean) => {
      current = { ...current, [item]: granted }
      return ok(state(current))
    })
    mocks.netlog = []
    mocks.dataExport.mockReset().mockImplementation(() => ok())
    mocks.save.mockReset()
  })

  it('采集内容说明与引导页是同一张表', async () => {
    const w = await mountPrivacy()
    const rows = w.findAll('table.notice tbody tr')
    expect(rows).toHaveLength(6)
    expect(rows[0]!.text()).toContain(t('onboarding.notice.rhythm.what'))
    expect(w.text()).toContain(t('onboarding.third_party'))
  })

  it('逐项列出六个同意项；撤回会发给第三方的一项时如实说明', async () => {
    const w = await mountPrivacy()
    expect(w.findAll('.consent')).toHaveLength(6)
    expect(w.find('[data-item=schedule]').text()).toContain('③')
    expect((box(w, 'jev_features').element as HTMLInputElement).checked).toBe(true)

    await box(w, 'jev_features').setValue(false)
    await flushPromises()
    expect(mocks.consentSet).toHaveBeenCalledWith('jev_features', false)
    expect(w.find('[role=status]').text()).toBe(t('error.withdraw_notice'))

    // 重新同意：说明收起
    await box(w, 'schedule').setValue(true)
    await flushPromises()
    expect(mocks.consentSet).toHaveBeenLastCalledWith('schedule', true)
    expect(w.find('.note').exists()).toBe(false)
  })

  it('撤回 ① 先确认；“先不撤回”什么也不改', async () => {
    const w = await mountPrivacy()
    await box(w, 'sense').setValue(false)
    expect(mocks.consentSet).not.toHaveBeenCalled()
    expect((box(w, 'sense').element as HTMLInputElement).checked).toBe(true)
    const confirm = w.find('[role=alertdialog]')
    expect(confirm.text()).toContain(t('privacy.sense_confirm'))
    await confirm.findAll('button')[1]!.trigger('click')
    expect(w.find('[role=alertdialog]').exists()).toBe(false)
    expect(mocks.consentSet).not.toHaveBeenCalled()

    await box(w, 'sense').setValue(false)
    await w.find('[role=alertdialog] button').trigger('click')
    await flushPromises()
    expect(mocks.consentSet).toHaveBeenCalledWith('sense', false)
    expect((box(w, 'sense').element as HTMLInputElement).checked).toBe(false)
    // ① 只在本机，不用“收不回来”的说明
    expect(w.find('.note').exists()).toBe(false)
  })

  it('出网记录：用途换成中文、列出字段名，不认识的用途和原因原样显示；没有记录时说明', async () => {
    mocks.netlog = [
      {
        ts: Date.UTC(2026, 9, 5, 12, 30, 5),
        api: 'llm/chat',
        model: 'deepseek-chat',
        fields: ['messages'],
        latency_ms: 812,
        status: 'ok',
        tokens_in: 120,
        tokens_out: 60,
      },
      {
        ts: Date.UTC(2026, 9, 5, 12, 29),
        api: 'jev',
        model: null,
        fields: ['kpm_z', 'backspace_rate'],
        latency_ms: null,
        status: 'timeout',
        tokens_in: null,
        tokens_out: null,
      },
      {
        ts: Date.UTC(2026, 9, 5, 12, 28),
        api: 'llm/new_scene',
        model: null,
        fields: [],
        latency_ms: 30,
        status: '401',
        tokens_in: null,
        tokens_out: null,
      },
    ]
    const w = await mountPrivacy()
    const rows = w.findAll('table.netlog tbody tr')
    expect(rows).toHaveLength(3)
    expect(rows[0]!.text()).toContain(t('privacy.api.llm_chat'))
    expect(rows[0]!.text()).toContain('deepseek-chat')
    expect(rows[0]!.text()).toContain('成功，812 毫秒')
    expect(rows[1]!.text()).toContain(t('privacy.api.jev'))
    expect(rows[1]!.text()).toContain('kpm_z、backspace_rate')
    expect(rows[1]!.text()).toContain('没成功（超时）')
    expect(rows[2]!.text()).toContain('llm/new_scene')
    expect(rows[2]!.find('.fields').text()).toBe(t('privacy.netlog_no_fields'))
    expect(rows[2]!.text()).toContain('没成功（401）')
    // 只有时间、用途、字段名、结果四列，没有请求或回复正文
    expect(w.findAll('table.netlog th')).toHaveLength(4)

    mocks.netlog = []
    await w.find('button.compact:not(.danger)').trigger('click')
    await flushPromises()
    expect(w.text()).toContain(t('privacy.netlog_empty'))
  })

  it('导出：选好位置后调 data_export，完成后显示路径；关掉对话框什么也不做', async () => {
    const w = await mountPrivacy()
    const btn = () => w.findAll('button').find((b) => b.text() === t('privacy.export_btn'))!

    mocks.save.mockResolvedValueOnce(null)
    await btn().trigger('click')
    await flushPromises()
    expect(mocks.dataExport).not.toHaveBeenCalled()

    mocks.save.mockResolvedValueOnce('D:\\备份\\心晴数据.zip')
    await btn().trigger('click')
    await flushPromises()
    const opts = mocks.save.mock.calls[1]![0]
    expect(opts.defaultPath).toMatch(/^心晴数据-\d{8}\.zip$/)
    expect(opts.filters[0].extensions).toEqual(['zip'])
    expect(mocks.dataExport).toHaveBeenCalledWith('D:\\备份\\心晴数据.zip')
    expect(w.find('.done').text()).toBe(t('error.export_done', { path: 'D:\\备份\\心晴数据.zip' }))
  })

  it('导出失败给出可读的原因，不显示错误码', async () => {
    mocks.save.mockResolvedValueOnce('/tmp/x.zip')
    mocks.dataExport.mockImplementationOnce(() =>
      Promise.resolve({ status: 'error', error: { code: 'data.export_failed', message_key: 'nope' } }),
    )
    const w = await mountPrivacy()
    await w
      .findAll('button')
      .find((b) => b.text() === t('privacy.export_btn'))!
      .trigger('click')
    await flushPromises()
    expect(w.find('[role=alert]').text()).toBe(t('error.generic'))
    expect(w.find('.done').exists()).toBe(false)
  })
})
