import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { ChatDelta, ChatDone, ChatErrorEvent, ChatMessageItem, ChatSent, ChatSessionItem } from '@/api'
import { t } from '@/i18n'
import { dayGroup } from './useChat'

type Listener<T> = (e: { payload: T }) => void
const mocks = vi.hoisted(() => ({
  sessions: [] as ChatSessionItem[],
  messages: {} as Record<number, ChatMessageItem[]>,
  chatSend: vi.fn(),
  chatRetry: vi.fn(),
  chatStop: vi.fn(),
  chatCopy: vi.fn(),
  chatDeleteAll: vi.fn(),
  safetyDismiss: vi.fn(),
  hide: vi.fn(),
  writeText: vi.fn(),
  delta: null as null | Listener<ChatDelta>,
  done: null as null | Listener<ChatDone>,
  error: null as null | Listener<ChatErrorEvent>,
  safety: null as null | Listener<{ session_id: number }>,
}))
const ok = (data: unknown = null) => Promise.resolve({ status: 'ok', data })
const { listen } = vi.hoisted(() => ({
  listen: (k: 'delta' | 'done' | 'error' | 'safety') => async (cb: unknown) => {
    ;(mocks as Record<string, unknown>)[k] = cb
    return () => {}
  },
}))

vi.mock('@/api', async (orig) => ({
  ...(await orig<typeof import('@/api')>()),
  commands: {
    chatListSessions: () => Promise.resolve({ status: 'ok', data: mocks.sessions }),
    chatGetMessages: (id: number) => Promise.resolve({ status: 'ok', data: mocks.messages[id] ?? [] }),
    chatSend: mocks.chatSend,
    chatRetry: mocks.chatRetry,
    chatStop: mocks.chatStop,
    chatCopy: mocks.chatCopy,
    chatDelete: () => Promise.resolve({ status: 'ok', data: null }),
    chatDeleteAll: mocks.chatDeleteAll,
    safetyDismiss: mocks.safetyDismiss,
  },
  events: {
    chatDelta: { listen: listen('delta') },
    chatDone: { listen: listen('done') },
    chatError: { listen: listen('error') },
    safetyTriggered: { listen: listen('safety') },
  },
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ hide: mocks.hide }) }))

const { default: App } = await import('./App.vue')

const NOW = Date.now()
const sent = (over: Partial<ChatSent> = {}): ChatSent => ({
  request_id: 7,
  session_id: 1,
  user_message_id: 10,
  new_session: true,
  safety: false,
  ...over,
})
const done = (over: Partial<ChatDone> = {}): ChatDone => ({
  request_id: 7,
  session_id: 1,
  message_id: 11,
  text: '听起来挺累的。',
  ai_generated: true,
  stopped: false,
  ...over,
})

async function mountChat() {
  const w = mount(App, { attachTo: document.body })
  await flushPromises()
  return w
}

async function type(w: Awaited<ReturnType<typeof mountChat>>, text: string) {
  await w.find('textarea').setValue(text)
  await w.find('textarea').trigger('keydown', { key: 'Enter' })
  await flushPromises()
}

describe('对话窗口', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    mocks.sessions = []
    mocks.messages = {}
    for (const f of [mocks.chatSend, mocks.chatRetry, mocks.chatStop, mocks.chatCopy, mocks.chatDeleteAll])
      f.mockReset().mockReturnValue(ok())
    mocks.chatSend.mockReturnValue(ok(sent()))
    mocks.safetyDismiss.mockReset().mockReturnValue(ok())
    mocks.writeText.mockReset().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: mocks.writeText },
      configurable: true,
    })
    document.body.innerHTML = ''
  })

  it('顶部常驻 AI 说明；6 小时内接着上次的会话，只有 AI 回复带“AI 生成”', async () => {
    mocks.sessions = [
      { id: 3, title: '考试', created_ts: NOW - 1000, last_ts: NOW - 1000, safe_mode: 'off', mode: 'normal' },
    ]
    mocks.messages[3] = [
      { id: 1, role: 'user', content: '考砸了', ts: NOW - 2000, ai_generated: false },
      { id: 2, role: 'assistant', content: '抱抱你。', ts: NOW - 1000, ai_generated: true },
      { id: 3, role: 'assistant', content: '固定回应', ts: NOW - 900, ai_generated: false },
    ]
    const w = await mountChat()
    expect(w.find('[role=note]').text()).toBe(t('chat.header_notice'))
    expect(w.findAll('.bubble').map((b) => b.text())).toEqual(['考砸了', '抱抱你。', '固定回应'])
    expect(w.findAll('.tag')).toHaveLength(1)
  })

  it('超过 6 小时的会话不自动打开', async () => {
    mocks.sessions = [
      { id: 3, title: '旧', created_ts: 0, last_ts: NOW - 7 * 3600_000, safe_mode: 'off', mode: 'normal' },
    ]
    mocks.messages[3] = [{ id: 1, role: 'user', content: '很久以前', ts: 0, ai_generated: false }]
    const w = await mountChat()
    expect(w.find('.empty').text()).toBe(t('chat.empty'))
  })

  it('Enter 发送，回复逐段显示，结束后以 chat:done 的全文为准', async () => {
    const w = await mountChat()
    await type(w, '  今天好累  ')
    expect(mocks.chatSend).toHaveBeenCalledWith(null, '今天好累', null)
    expect(w.find('.typing').text()).toBe(t('chat.typing'))
    expect(w.find('button.compact').exists()).toBe(true)
    expect(w.text()).toContain(t('chat.btn_stop'))
    mocks.delta!({ payload: { request_id: 7, text_delta: '听起来' } })
    await flushPromises()
    expect(w.findAll('.bubble').map((b) => b.text())).toEqual(['今天好累', '听起来'])
    mocks.done!({ payload: done() })
    await flushPromises()
    expect(w.findAll('.bubble')[1]!.text()).toBe('听起来挺累的。')
    expect(w.find('.tag').text()).toBe(t('chat.ai_tag'))
    expect(w.find('textarea').element.value).toBe('')
  })

  it('Shift+Enter 换行、输入法组字中的 Enter 都不发送', async () => {
    const w = await mountChat()
    await w.find('textarea').setValue('你好')
    await w.find('textarea').trigger('keydown', { key: 'Enter', shiftKey: true })
    await w.find('textarea').trigger('keydown', { key: 'Enter', isComposing: true })
    await flushPromises()
    expect(mocks.chatSend).not.toHaveBeenCalled()
  })

  it('chat_send 返回之前到达的增量不会丢', async () => {
    let resolve!: (v: unknown) => void
    mocks.chatSend.mockReturnValue(new Promise((r) => (resolve = r)))
    const w = await mountChat()
    await type(w, '在吗')
    mocks.delta!({ payload: { request_id: 7, text_delta: '在' } })
    mocks.done!({ payload: done({ text: '在的。' }) })
    resolve({ status: 'ok', data: sent() })
    await flushPromises()
    expect(w.findAll('.bubble').map((b) => b.text())).toEqual(['在吗', '在的。'])
  })

  it('失败时移除半截回复，保留用户消息，可以重试', async () => {
    const w = await mountChat()
    await type(w, '你好')
    mocks.delta!({ payload: { request_id: 7, text_delta: '半截' } })
    mocks.error!({ payload: { request_id: 7, session_id: 1, reason: 'stuck' } })
    await flushPromises()
    expect(w.findAll('.bubble').map((b) => b.text())).toEqual(['你好'])
    expect(w.find('.failure').text()).toContain(t('chat.stuck'))
    mocks.chatRetry.mockReturnValue(ok(sent({ request_id: 8, new_session: false, user_message_id: null })))
    await w.find('.failure button').trigger('click')
    await flushPromises()
    expect(mocks.chatRetry).toHaveBeenCalledWith(1)
    mocks.done!({ payload: done({ request_id: 8, text: '你好呀。' }) })
    await flushPromises()
    expect(w.findAll('.bubble').map((b) => b.text())).toEqual(['你好', '你好呀。'])
  })

  it('额度用完给对应提示', async () => {
    const w = await mountChat()
    await type(w, '你好')
    mocks.error!({ payload: { request_id: 7, session_id: 1, reason: 'daily_cap' } })
    await flushPromises()
    expect(w.find('.failure').text()).toContain(t('chat.daily_cap'))
  })

  it('复制 AI 回复用后端给的带标识文本（FR-CHT-04 第 5 条）', async () => {
    mocks.chatCopy.mockReturnValue(ok('抱抱你。（内容由 AI 生成）'))
    mocks.sessions = [{ id: 3, title: 'x', created_ts: NOW, last_ts: NOW, safe_mode: 'off', mode: 'normal' }]
    mocks.messages[3] = [{ id: 2, role: 'assistant', content: '抱抱你。', ts: NOW, ai_generated: true }]
    const w = await mountChat()
    await w.find('.meta button').trigger('click')
    await flushPromises()
    expect(mocks.chatCopy).toHaveBeenCalledWith(2)
    expect(mocks.writeText).toHaveBeenCalledWith('抱抱你。（内容由 AI 生成）')
    expect(w.find('.meta button').text()).toBe(t('chat.copied'))
  })

  it('Esc 隐藏窗口', async () => {
    const w = await mountChat()
    await w.find('main').trigger('keydown', { key: 'Escape' })
    expect(mocks.hide).toHaveBeenCalled()
  })

  describe('求助卡片（FR-SAF-02、FR-SAF-06）', () => {
    it('本地词表命中：立即显示卡片，号码可复制；“我现在是安全的”折叠成一行但不移除', async () => {
      mocks.chatSend.mockReturnValue(ok(sent({ safety: true })))
      const w = await mountChat()
      await type(w, '我真的不想活了')
      const card = w.find('[role=alert]')
      expect(card.text()).toContain('12356')
      expect(card.text()).toContain(t('safety.school_phone_empty'))
      await card.findAll('.numbers button')[0]!.trigger('click')
      await flushPromises()
      expect(mocks.writeText).toHaveBeenCalledWith('12356')
      await w.findAll('.actions button')[0]!.trigger('click')
      expect(w.find('[role=alert]').exists()).toBe(false)
      expect(w.find('.safety.collapsed').text()).toContain(t('safety.collapsed'))
    })

    it('Jev 后到的 safety:triggered 也会显示卡片', async () => {
      const w = await mountChat()
      await type(w, '一切都没意义')
      expect(w.find('.safety').exists()).toBe(false)
      mocks.safety!({ payload: { session_id: 1 } })
      await flushPromises()
      expect(w.find('[role=alert]').exists()).toBe(true)
    })

    it('“我说的不是这个意思”：通知后端，卡片折叠保留，不再出现这个按钮', async () => {
      mocks.chatSend.mockReturnValue(ok(sent({ safety: true })))
      const w = await mountChat()
      await type(w, '我真的不想活了')
      await w.findAll('.actions button')[1]!.trigger('click')
      await flushPromises()
      expect(mocks.safetyDismiss).toHaveBeenCalledWith(1)
      expect(w.find('.safety.collapsed').exists()).toBe(true)
      await w.find('.safety.collapsed button').trigger('click')
      expect(w.text()).not.toContain(t('safety.btn_misread'))
    })

    it('打开触发过危机识别的会话时卡片在顶部', async () => {
      mocks.sessions = [{ id: 3, title: 'x', created_ts: NOW, last_ts: NOW, safe_mode: 'on', mode: 'normal' }]
      const w = await mountChat()
      expect(w.find('[role=alert]').exists()).toBe(true)
    })
  })

  it('历史抽屉按日期分组；删除全部要再确认一次', async () => {
    mocks.sessions = [
      {
        id: 1,
        title: '今天的',
        created_ts: NOW,
        last_ts: NOW - 8 * 3600_000,
        safe_mode: 'off',
        mode: 'normal',
      },
      { id: 2, title: '很早的', created_ts: 0, last_ts: 0, safe_mode: 'off', mode: 'normal' },
    ]
    const w = await mountChat()
    await w.find('.bar .icon').trigger('click')
    const heads = w.findAll('.drawer h2').map((h) => h.text())
    expect(heads.at(-1)).toBe(t('chat.earlier'))
    expect(w.findAll('.drawer .item').map((b) => b.text())).toEqual(['今天的', '很早的'])
    await w.find('.danger button').trigger('click')
    expect(mocks.chatDeleteAll).not.toHaveBeenCalled()
    await w.find('.danger button').trigger('click')
    await flushPromises()
    expect(mocks.chatDeleteAll).toHaveBeenCalled()
  })
})

describe('dayGroup', () => {
  it('今天 / 昨天 / 更早', () => {
    const noon = new Date(2026, 2, 10, 12).getTime()
    expect(dayGroup(new Date(2026, 2, 10, 0, 1).getTime(), noon)).toBe('today')
    expect(dayGroup(new Date(2026, 2, 9, 23).getTime(), noon)).toBe('yesterday')
    expect(dayGroup(new Date(2026, 2, 8, 23).getTime(), noon)).toBe('earlier')
  })
})
