// 对话窗口的状态（05 FR-CHT-02/04、FR-SAF-02/06）：会话列表、当前会话的消息、流式回复、求助卡片。
// 回复经 chat:delta / chat:done / chat:error 推来；chat_send 返回 request_id 之前就可能有事件到达，先按 request_id 暂存。
import { computed, onBeforeUnmount, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  commands,
  events,
  unwrap,
  type ChatDone,
  type ChatErrorEvent,
  type ChatFailure,
  type ChatSent,
  type ChatSessionItem,
  type SafeMode,
} from '@/api'

/** 距上一条消息超过 6 小时就开新会话（FR-CHT-02 第 1 条），打开窗口时据此决定是否接着上次聊。 */
export const NEW_SESSION_GAP_MS = 6 * 60 * 60 * 1000
/** 单条消息最多 2000 字（FR-CHT-09）。 */
export const MAX_INPUT = 2000

export type ChatLine = {
  /** 写库后的消息 id；正在生成的回复没有 */
  id: number | null
  role: 'user' | 'assistant'
  content: string
  aiGenerated: boolean
  /** 正在流式生成 */
  pending?: boolean
}

type Pending = { requestId: number | null; sessionId: number | null }
type Buffered = { delta?: string; done?: ChatDone; error?: ChatErrorEvent }

/** 抽屉里的日期分组（FR-CHT-02 第 3 条）。 */
export function dayGroup(ts: number, now: number): 'today' | 'yesterday' | 'earlier' {
  const start = new Date(now)
  start.setHours(0, 0, 0, 0)
  const today = start.getTime()
  if (ts >= today) return 'today'
  if (ts >= today - 24 * 60 * 60 * 1000) return 'yesterday'
  return 'earlier'
}

export function useChat(now: () => number = Date.now) {
  const sessions = ref<ChatSessionItem[]>([])
  const sessionId = ref<number | null>(null)
  const lines = ref<ChatLine[]>([])
  const safeMode = ref<SafeMode>('off')
  /** 点了“我现在是安全的”：卡片折叠成一行，本会话内仍可见、不可移除（FR-SAF-02 第 1 条） */
  const collapsed = ref(false)
  const failure = ref<ChatFailure | null>(null)
  const pending = ref<Pending | null>(null)
  const busy = computed(() => pending.value !== null)

  // 先于 chat_send 返回到达的事件
  const early = new Map<number, Buffered[]>()
  // 收到过 safety:triggered 的会话（可能在 chat_send 返回前到达）
  const triggered = new Set<number>()
  const unlisten: UnlistenFn[] = []

  function showSafety(id: number): void {
    triggered.add(id)
    if (sessionId.value === id || (sessionId.value === null && pending.value)) {
      safeMode.value = 'on'
      collapsed.value = false
    }
  }

  function apply(ev: Buffered): void {
    const last = lines.value[lines.value.length - 1]
    if (ev.delta !== undefined) {
      if (last?.pending) last.content += ev.delta
      return
    }
    if (ev.done) {
      const d = ev.done
      if (last?.pending) {
        if (d.message_id === null) lines.value.pop()
        else
          Object.assign(last, {
            id: d.message_id,
            content: d.text,
            aiGenerated: d.ai_generated,
            pending: false,
          })
      }
      pending.value = null
      void refreshSessions()
      return
    }
    if (ev.error) {
      // 半截回复丢掉，用户消息保留，显示提示和“重试”（FR-CHT-04 第 2 条）
      if (last?.pending) lines.value.pop()
      failure.value = ev.error.reason
      pending.value = null
    }
  }

  function route(requestId: number, ev: Buffered): void {
    const p = pending.value
    if (p?.requestId === requestId) apply(ev)
    else if (p && p.requestId === null) {
      const list = early.get(requestId) ?? []
      list.push(ev)
      early.set(requestId, list)
    }
  }

  async function init(): Promise<void> {
    unlisten.push(
      await events.chatDelta.listen((e) => route(e.payload.request_id, { delta: e.payload.text_delta })),
      await events.chatDone.listen((e) => route(e.payload.request_id, { done: e.payload })),
      await events.chatError.listen((e) => route(e.payload.request_id, { error: e.payload })),
      await events.safetyTriggered.listen((e) => showSafety(e.payload.session_id)),
    )
    await refreshSessions()
    // 打开窗口时接着最近的会话，除非已经过了 6 小时
    const latest = sessions.value[0]
    if (latest && now() - (latest.last_ts ?? 0) <= NEW_SESSION_GAP_MS) await open(latest.id)
  }

  async function refreshSessions(): Promise<void> {
    sessions.value = await unwrap(commands.chatListSessions())
  }

  async function open(id: number): Promise<void> {
    const msgs = await unwrap(commands.chatGetMessages(id))
    sessionId.value = id
    lines.value = msgs
      .filter((m) => m.role === 'user' || m.role === 'assistant')
      .map((m) => ({
        id: m.id,
        role: m.role as ChatLine['role'],
        content: m.content,
        aiGenerated: m.ai_generated,
      }))
    const s = sessions.value.find((x) => x.id === id)
    safeMode.value = triggered.has(id) ? 'on' : (s?.safe_mode ?? 'off')
    collapsed.value = safeMode.value === 'dismissed'
    failure.value = null
  }

  /** “新对话”（FR-CHT-02 第 1 条）：下一条消息开新会话。 */
  function newChat(): void {
    sessionId.value = null
    lines.value = []
    safeMode.value = 'off'
    collapsed.value = false
    failure.value = null
  }

  async function start(call: Promise<ChatSent>): Promise<void> {
    pending.value = { requestId: null, sessionId: sessionId.value }
    lines.value.push({ id: null, role: 'assistant', content: '', aiGenerated: true, pending: true })
    let sent: ChatSent
    try {
      sent = await call
    } catch (e) {
      lines.value.pop()
      pending.value = null
      throw e
    }
    sessionId.value = sent.session_id
    if (sent.safety || triggered.has(sent.session_id)) showSafety(sent.session_id)
    else if (sent.new_session) safeMode.value = 'off'
    pending.value = { requestId: sent.request_id, sessionId: sent.session_id }
    for (const ev of early.get(sent.request_id) ?? []) apply(ev)
    early.clear()
  }

  async function send(text: string): Promise<void> {
    const body = text.trim()
    if (!body || busy.value) return
    failure.value = null
    lines.value.push({ id: null, role: 'user', content: body, aiGenerated: false })
    const userLine = lines.value[lines.value.length - 1]!
    try {
      await start(
        unwrap(commands.chatSend(sessionId.value, body)).then((s) => {
          userLine.id = s.user_message_id
          if (s.new_session && sessionId.value !== null) {
            // 上一条已超过 6 小时：后端开了新会话，界面也只留这一条
            lines.value = lines.value.slice(-2)
          }
          return s
        }),
      )
    } catch (e) {
      lines.value = lines.value.filter((l) => l !== userLine)
      throw e
    }
  }

  async function retry(): Promise<void> {
    if (sessionId.value === null || busy.value) return
    failure.value = null
    await start(unwrap(commands.chatRetry(sessionId.value)))
  }

  async function stop(): Promise<void> {
    const id = pending.value?.requestId
    if (id != null) await unwrap(commands.chatStop(id))
  }

  /** 复制一条回复：AI 回复带“（内容由 AI 生成）”（FR-CHT-04 第 5 条）。 */
  async function copy(line: ChatLine): Promise<void> {
    const text = line.id === null ? line.content : await unwrap(commands.chatCopy(line.id))
    await navigator.clipboard.writeText(text)
  }

  /** “我说的不是这个意思”（FR-SAF-06）：回到普通模式，求助信息折叠保留。 */
  async function dismiss(): Promise<void> {
    if (sessionId.value === null) return
    await unwrap(commands.safetyDismiss(sessionId.value))
    triggered.delete(sessionId.value)
    safeMode.value = 'dismissed'
    collapsed.value = true
  }

  async function remove(id: number): Promise<void> {
    await unwrap(commands.chatDelete(id))
    if (sessionId.value === id) newChat()
    await refreshSessions()
  }

  async function removeAll(): Promise<void> {
    await unwrap(commands.chatDeleteAll())
    newChat()
    await refreshSessions()
  }

  onBeforeUnmount(() => unlisten.forEach((u) => u()))

  return {
    sessions,
    sessionId,
    lines,
    safeMode,
    collapsed,
    failure,
    busy,
    init,
    open,
    newChat,
    send,
    retry,
    stop,
    copy,
    dismiss,
    remove,
    removeAll,
    refreshSessions,
  }
}
