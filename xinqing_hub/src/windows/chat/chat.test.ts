import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import type { ChatDelta, ChatDone, ChatErrorEvent, ChatMessageItem, ChatSent, ChatSessionItem } from '@/api'
import { t } from '@/i18n'
import { TOTAL_SECONDS } from './breathing'
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
  chatSetMode: vi.fn(),
  memoryAdd: vi.fn(),
  /** 设置“关怀”里填的学校心理中心电话 */
  schoolPhone: '',
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
    chatSetMode: mocks.chatSetMode,
    memoryAdd: mocks.memoryAdd,
    settingsGet: (key: string) =>
      Promise.resolve({ status: 'ok', data: key === 'safety.school_phone' ? mocks.schoolPhone : 'system' }),
  },
  events: {
    chatDelta: { listen: listen('delta') },
    chatDone: { listen: listen('done') },
    chatError: { listen: listen('error') },
    safetyTriggered: { listen: listen('safety') },
    settingsChanged: { listen: async () => () => {} },
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
  memory_candidate: null,
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
    mocks.schoolPhone = ''
    for (const f of [mocks.chatSend, mocks.chatRetry, mocks.chatStop, mocks.chatCopy, mocks.chatDeleteAll])
      f.mockReset().mockReturnValue(ok())
    mocks.chatSend.mockReturnValue(ok(sent()))
    mocks.safetyDismiss.mockReset().mockReturnValue(ok())
    mocks.chatSetMode.mockReset().mockReturnValue(ok())
    mocks.memoryAdd.mockReset().mockReturnValue(ok(1))
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

  it('呼吸引导：焦点移到“结束”，Esc 只关引导、不隐藏窗口，焦点回到输入框（DS-A11Y-01）', async () => {
    mocks.hide.mockClear()
    const w = await mountChat()
    await w.findAll('.shortcuts button')[2]!.trigger('click')
    await flushPromises()
    expect(document.activeElement).toBe(w.find('.breathing button').element)
    await w.find('main').trigger('keydown', { key: 'Escape' })
    expect(w.find('.breathing').exists()).toBe(false)
    expect(mocks.hide).not.toHaveBeenCalled()
    expect(document.activeElement).toBe(w.find('textarea').element)
    w.unmount()
  })

  it('历史抽屉：打开时焦点进抽屉，Esc 关上后回到“历史”按钮', async () => {
    mocks.sessions = [
      { id: 1, title: '今天的', created_ts: NOW, last_ts: NOW, safe_mode: 'off', mode: 'normal' },
    ]
    const w = await mountChat()
    await w.find('.bar .icon').trigger('click')
    await flushPromises()
    expect(document.activeElement).toBe(w.find('.drawer .item').element)
    await w.find('main').trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(w.find('.drawer').exists()).toBe(false)
    expect(document.activeElement).toBe(w.find('.bar .icon').element)
    w.unmount()
  })

  it('“晴晴正在输入…”带三个点的动画，读屏只读文字', async () => {
    const w = await mountChat()
    await type(w, '在吗')
    const typing = w.find('.typing')
    expect(typing.findAll('.dots span')).toHaveLength(3)
    expect(typing.find('.dots').attributes('aria-hidden')).toBe('true')
    w.unmount()
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

    it('设置里填了学校心理中心电话：卡片上显示这个号码，也能一键复制（FR-SAF-03）', async () => {
      mocks.schoolPhone = '0571-8888 1234'
      mocks.chatSend.mockReturnValue(ok(sent({ safety: true })))
      const w = await mountChat()
      await type(w, '我真的不想活了')
      await flushPromises()
      const card = w.find('[role=alert]')
      expect(card.text()).toContain('学校心理中心：0571-8888 1234')
      expect(card.text()).not.toContain(t('safety.school_phone_empty'))
      const buttons = card.findAll('.numbers button')
      expect(buttons).toHaveLength(4)
      await buttons[3]!.trigger('click')
      await flushPromises()
      expect(mocks.writeText).toHaveBeenCalledWith('0571-8888 1234')
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

describe('记忆与快捷指令', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    mocks.sessions = []
    mocks.messages = {}
    mocks.chatSend.mockReset().mockReturnValue(ok(sent()))
    mocks.chatSetMode.mockReset().mockReturnValue(ok())
    mocks.memoryAdd.mockReset().mockReturnValue(ok(1))
    document.body.innerHTML = ''
  })

  it('对话里明确说“记住”时先问，点“记住”才写入（FR-CHT-07 第 1 条）', async () => {
    mocks.chatSend.mockReturnValue(ok(sent({ memory_candidate: '下周三考英语' })))
    const w = await mountChat()
    await type(w, '帮我记一下：下周三考英语')
    expect(w.find('.ask').text()).toContain(t('chat.remember_ask'))
    expect(w.find('.ask blockquote').text()).toBe('下周三考英语')
    expect(mocks.memoryAdd).not.toHaveBeenCalled()
    await w.find('.ask .primary').trigger('click')
    await flushPromises()
    expect(mocks.memoryAdd).toHaveBeenCalledWith('下周三考英语')
    expect(w.find('.ask').exists()).toBe(false)
    expect(w.find('.info').text()).toBe(t('chat.remember_done'))
  })

  it('消息上的“让晴晴记住”也先确认；“不用了”就不写；满 50 条时说清楚', async () => {
    mocks.sessions = [{ id: 3, title: 'x', created_ts: NOW, last_ts: NOW, safe_mode: 'off', mode: 'normal' }]
    mocks.messages[3] = [{ id: 1, role: 'user', content: '我对芒果过敏', ts: NOW, ai_generated: false }]
    const w = await mountChat()
    await w.find('.remember').trigger('click')
    expect(w.find('.ask blockquote').text()).toBe('我对芒果过敏')
    await w.findAll('.ask button')[1]!.trigger('click')
    expect(w.find('.ask').exists()).toBe(false)
    expect(mocks.memoryAdd).not.toHaveBeenCalled()
    mocks.memoryAdd.mockReturnValue(
      Promise.resolve({
        status: 'error',
        error: { code: 'chat.memory_full', message_key: 'error.memory_full' },
      }),
    )
    await w.find('.remember').trigger('click')
    await w.find('.ask .primary').trigger('click')
    await flushPromises()
    expect(w.find('.error').text()).toBe(t('error.memory_full'))
  })

  it('还没有会话时选“我只是想吐槽”，随第一条消息带上；之后可以回到平常（FR-CHT-06）', async () => {
    const w = await mountChat()
    await w.findAll('.shortcuts button')[0]!.trigger('click')
    await flushPromises()
    expect(mocks.chatSetMode).not.toHaveBeenCalled()
    expect(w.find('.mode').text()).toContain(t('chat.mode_vent'))
    await type(w, '今天被说了一顿')
    expect(mocks.chatSend).toHaveBeenCalledWith(null, '今天被说了一顿', 'vent')
    mocks.done!({ payload: done() })
    await flushPromises()
    await w.find('.mode button').trigger('click')
    await flushPromises()
    expect(mocks.chatSetMode).toHaveBeenCalledWith(1, 'normal')
    expect(w.find('.mode').exists()).toBe(false)
  })

  it('打开会话时显示它的对话方式', async () => {
    mocks.sessions = [
      { id: 3, title: 'x', created_ts: NOW, last_ts: NOW, safe_mode: 'off', mode: 'organize' },
    ]
    const w = await mountChat()
    expect(w.find('.mode').text()).toContain(t('chat.mode_organize'))
  })

  it('陪我呼吸：吸 4 秒、停 4 秒、呼 6 秒，共 4 轮，不调用后端', async () => {
    vi.useFakeTimers()
    try {
      const w = await mountChat()
      await w.findAll('.shortcuts button')[2]!.trigger('click')
      const label = () => w.find('.breathing .label').text()
      expect(label()).toBe(t('chat.breathe_in'))
      await vi.advanceTimersByTimeAsync(4000)
      expect(label()).toBe(t('chat.breathe_hold'))
      await vi.advanceTimersByTimeAsync(4000)
      expect(label()).toBe(t('chat.breathe_out'))
      await vi.advanceTimersByTimeAsync(6000)
      expect(w.find('.breathing .round').text()).toBe(t('chat.breathe_round', { n: 2 }))
      await vi.advanceTimersByTimeAsync((TOTAL_SECONDS - 14) * 1000)
      expect(label()).toBe(t('chat.breathe_done'))
      expect(mocks.chatSend).not.toHaveBeenCalled()
      await w.find('.breathing button').trigger('click')
      expect(w.find('.breathing').exists()).toBe(false)
    } finally {
      vi.useRealTimers()
    }
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
