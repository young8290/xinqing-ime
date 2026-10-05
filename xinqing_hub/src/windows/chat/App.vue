<script setup lang="ts">
// 对话窗口（05 FR-CHT-01/02/04/09，16 D-06）：顶部常驻 AI 说明，危机时固定求助卡片（FR-SAF-02），
// 左侧抽屉是按日期分组的历史会话，回复逐字显示并带 `AI 生成` 标签，可停止、重试、复制。
// 快捷指令（FR-CHT-06）：吐槽 / 理一理用 `chatSetMode` 或 `chatSend` 的 `mode` 切换会话的对话方式（ADR 0022），
// 日记与呼吸是窗口本地动作，按钮随后续 PR 补上；记忆（FR-CHT-07）、历史搜索（FR-CHT-08）也是 P1。
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { errorText, t } from '@/i18n'
import SafetyCard from './SafetyCard.vue'
import { MAX_INPUT, dayGroup, useChat, type ChatLine } from './useChat'

const chat = useChat()
const draft = ref('')
const notice = ref<string | null>(null)
const drawer = ref(false)
const confirmAll = ref(false)
const copiedId = ref<ChatLine | null>(null)
const logEl = ref<HTMLElement | null>(null)
const inputEl = ref<HTMLTextAreaElement | null>(null)

const groups = computed(() => {
  const now = Date.now()
  const out = { today: [], yesterday: [], earlier: [] } as Record<
    'today' | 'yesterday' | 'earlier',
    typeof chat.sessions.value
  >
  for (const s of chat.sessions.value) out[dayGroup(s.last_ts ?? 0, now)].push(s)
  return (['today', 'yesterday', 'earlier'] as const)
    .map((k) => ({ key: k, items: out[k] }))
    .filter((g) => g.items.length > 0)
})

const failureText = computed(() =>
  chat.failure.value === 'daily_cap' ? t('chat.daily_cap') : chat.failure.value ? t('chat.stuck') : null,
)
const showSafety = computed(() => chat.safeMode.value !== 'off')

async function run(action: () => Promise<unknown>): Promise<void> {
  notice.value = null
  try {
    await action()
  } catch (e) {
    notice.value = errorText(e)
  }
}

onMounted(() =>
  run(async () => {
    await chat.init()
    inputEl.value?.focus()
  }),
)

// 有新内容时滚到底
watch(
  () => [chat.lines.value.length, chat.lines.value[chat.lines.value.length - 1]?.content],
  async () => {
    await nextTick()
    if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight
  },
)

async function submit(): Promise<void> {
  const text = draft.value
  if (!text.trim() || chat.busy.value) return
  draft.value = ''
  await run(() => chat.send(text))
  if (notice.value) draft.value = text
}

function onInputKeydown(e: KeyboardEvent): void {
  // Enter 发送、Shift+Enter 换行（FR-CHT-09）；输入法正在组字时的 Enter 是上屏，不发送
  if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return
  e.preventDefault()
  void submit()
}

function onKeydown(e: KeyboardEvent): void {
  // Esc 关闭窗口，不结束会话（FR-CHT-01）；抽屉开着时先关抽屉
  if (e.key !== 'Escape') return
  if (drawer.value) drawer.value = false
  else void getCurrentWindow().hide()
}

async function openSession(id: number): Promise<void> {
  drawer.value = false
  await run(() => chat.open(id))
}

function newChat(): void {
  drawer.value = false
  chat.newChat()
  inputEl.value?.focus()
}

async function copy(line: ChatLine): Promise<void> {
  await run(() => chat.copy(line))
  if (!notice.value) copiedId.value = line
}

async function removeAll(): Promise<void> {
  confirmAll.value = false
  drawer.value = false
  await run(() => chat.removeAll())
}
</script>

<template>
  <main class="chat" @keydown="onKeydown">
    <p class="notice" role="note">{{ t('chat.header_notice') }}</p>
    <header class="bar">
      <button
        class="icon"
        :aria-label="t('chat.history')"
        :title="t('chat.history')"
        :aria-expanded="drawer"
        @click="drawer = !drawer"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2 4h12M2 8h12M2 12h12" />
        </svg>
      </button>
      <h1>{{ t('window.chat') }}</h1>
      <button class="compact" :disabled="chat.busy.value" @click="newChat">{{ t('chat.new_chat') }}</button>
    </header>

    <SafetyCard
      v-if="showSafety"
      :collapsed="chat.collapsed.value"
      :dismissed="chat.safeMode.value === 'dismissed'"
      @safe="chat.collapsed.value = true"
      @expand="chat.collapsed.value = false"
      @misread="run(chat.dismiss)"
    />

    <div ref="logEl" class="log" role="log" aria-live="polite">
      <p v-if="chat.lines.value.length === 0" class="empty">{{ t('chat.empty') }}</p>
      <div v-for="(l, i) in chat.lines.value" :key="l.id ?? `p${i}`" class="row" :class="l.role">
        <p v-if="l.pending && !l.content" class="typing">{{ t('chat.typing') }}</p>
        <p v-else class="bubble">{{ l.content }}</p>
        <div v-if="l.role === 'assistant' && !l.pending" class="meta">
          <span v-if="l.aiGenerated" class="tag">{{ t('chat.ai_tag') }}</span>
          <button class="link" @click="copy(l)">
            {{ copiedId === l ? t('chat.copied') : t('chat.copy') }}
          </button>
        </div>
      </div>
      <div v-if="failureText" class="failure" role="status">
        <span>{{ failureText }}</span>
        <button class="compact" @click="run(chat.retry)">{{ t('chat.btn_retry') }}</button>
      </div>
    </div>

    <p v-if="notice" class="error" role="alert">{{ notice }}</p>
    <form class="composer" @submit.prevent="submit">
      <textarea
        ref="inputEl"
        v-model="draft"
        rows="2"
        :maxlength="MAX_INPUT"
        :placeholder="t('chat.placeholder')"
        :aria-label="t('chat.placeholder')"
        @keydown="onInputKeydown"
      />
      <button v-if="chat.busy.value" type="button" class="compact" @click="run(chat.stop)">
        {{ t('chat.btn_stop') }}
      </button>
      <button v-else type="submit" class="compact primary" :disabled="!draft.trim()">
        {{ t('chat.send') }}
      </button>
    </form>

    <nav v-if="drawer" class="drawer" :aria-label="t('chat.history')">
      <p v-if="groups.length === 0" class="muted">{{ t('chat.history_empty') }}</p>
      <section v-for="g in groups" :key="g.key">
        <h2>{{ t(`chat.${g.key}`) }}</h2>
        <ul>
          <li v-for="s in g.items" :key="s.id" :class="{ current: s.id === chat.sessionId.value }">
            <button class="item" @click="openSession(s.id)">{{ s.title }}</button>
            <button
              class="icon"
              :aria-label="t('chat.delete')"
              :title="t('chat.delete')"
              @click="run(() => chat.remove(s.id))"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
                <path d="M4 4l8 8M12 4l-8 8" />
              </svg>
            </button>
          </li>
        </ul>
      </section>
      <div v-if="groups.length > 0" class="danger">
        <template v-if="confirmAll">
          <p>{{ t('chat.delete_all_confirm') }}</p>
          <button class="compact" @click="removeAll">{{ t('chat.confirm') }}</button>
          <button class="compact" @click="confirmAll = false">{{ t('chat.cancel') }}</button>
        </template>
        <button v-else class="compact" @click="confirmAll = true">{{ t('chat.delete_all') }}</button>
      </div>
    </nav>
  </main>
</template>

<style scoped>
.chat {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100vh;
}

/* 顶部常驻 AI 说明（FR-CHT-04 第 4 条、DS-COPY-05） */
.notice {
  margin: 0;
  padding: var(--xq-sp-2) var(--xq-sp-4);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.bar {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  padding: var(--xq-sp-2) var(--xq-sp-3);
}

.bar h1 {
  flex: 1;
  margin: 0;
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.icon {
  flex: none;
  padding: 0;
  border-color: transparent;
  background: transparent;
  color: var(--xq-text-2);
}

.icon path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
}

.compact {
  padding: 0 var(--xq-sp-2);
  font-size: var(--xq-fs-xs);
}

.log {
  display: flex;
  flex: 1;
  flex-direction: column;
  gap: var(--xq-sp-3);
  padding: var(--xq-sp-2) var(--xq-sp-4);
  overflow-y: auto;
}

.empty {
  margin: auto;
  color: var(--xq-text-2);
}

.row {
  display: flex;
  flex-direction: column;
  max-width: 85%;
}

.row.user {
  align-self: flex-end;
  align-items: flex-end;
}

.row.assistant {
  align-self: flex-start;
}

.bubble,
.typing {
  margin: 0;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-card);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  user-select: text;
}

.user .bubble {
  background: var(--xq-primary);
  color: var(--xq-on-primary);
}

.assistant .bubble {
  background: var(--xq-surface-2);
}

.typing {
  color: var(--xq-text-2);
}

.meta {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  margin-top: var(--xq-sp-1);
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.tag {
  padding: 0 var(--xq-sp-1);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
}

.link {
  min-width: 0;
  min-height: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--xq-link);
  font-size: var(--xq-fs-xs);
}

.failure {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.error {
  margin: 0 var(--xq-sp-4);
  color: var(--xq-danger);
  font-size: var(--xq-fs-sm);
}

.composer {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: flex-end;
  padding: var(--xq-sp-3) var(--xq-sp-4) var(--xq-sp-4);
  border-top: 1px solid var(--xq-border);
}

textarea {
  flex: 1;
  min-height: 48px;
  max-height: 160px;
  padding: var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  color: var(--xq-text-1);
  font: inherit;
  resize: vertical;
}

.drawer {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  z-index: 2;
  width: 260px;
  padding: var(--xq-sp-4) var(--xq-sp-3);
  overflow-y: auto;
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-float);
}

.drawer h2 {
  margin: var(--xq-sp-3) 0 var(--xq-sp-1);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.drawer ul {
  margin: 0;
  padding: 0;
  list-style: none;
}

.drawer li {
  display: flex;
  align-items: center;
  border-radius: var(--xq-radius-sm);
}

.drawer li.current {
  background: var(--xq-surface-2);
}

.item {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  border-color: transparent;
  background: transparent;
  text-align: left;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.danger {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-1);
  margin-top: var(--xq-sp-4);
  font-size: var(--xq-fs-sm);
}

.danger p {
  width: 100%;
  margin: 0;
}
</style>
