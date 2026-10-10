<script setup lang="ts">
// 一句话区（07 FR-WGT-04）：最多 2 行、超出省略、悬停看全文；暖心话逐字出现（每字 40 ms，减少动态效果时整句显示，
// FR-CMF-04 第 1 条），大模型写的句尾单独一行标 `AI 生成`（不会被省略号截掉，FR-CMF-04 验收：100% 带标识）；
// 悬停或键盘聚焦时出现 👍 / 👎 / 🔕（FR-CMF-05）。带动作的消息（求助入口、周信提示）整句是一个按钮。
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { ComfortVerdict } from '@/api'
import { t } from '@/i18n'
import type { Message } from './messages'

const props = defineProps<{ msg: Message | null }>()
const emit = defineEmits<{ act: [msg: Message]; vote: [id: number, verdict: ComfortVerdict] }>()

/** 打字机效果每个字的间隔（FR-CMF-04 第 1 条） */
const CHAR_MS = 40
/** 只有刚到的暖心话才逐字出现；打开窗口时补上的“今日一句”直接整句显示 */
const FRESH_MS = 5_000

const typed = ref<number | null>(null)
let timer: ReturnType<typeof setInterval> | undefined

function reducedMotion(): boolean {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
}

watch(
  () => props.msg,
  (m, old) => {
    clearInterval(timer)
    typed.value = null
    const fresh = m?.kind === 'comfort' && Date.now() - m.at < FRESH_MS
    if (!m || !fresh || m === old || reducedMotion()) return
    const chars = [...m.text]
    typed.value = 0
    timer = setInterval(() => {
      typed.value = (typed.value ?? 0) + 1
      if (typed.value >= chars.length) {
        clearInterval(timer)
        typed.value = null
      }
    }, CHAR_MS)
  },
  { immediate: true },
)

onBeforeUnmount(() => clearInterval(timer))

const text = computed(() => {
  const m = props.msg
  if (!m) return ''
  return typed.value === null ? m.text : [...m.text].slice(0, typed.value).join('')
})

/** 已经反馈过的暖心话（按行号记）：按钮换成致谢 */
const voted = ref<{ id: number; verdict: ComfortVerdict } | null>(null)
const canVote = computed(() => props.msg?.comfortId != null && voted.value?.id !== props.msg.comfortId)
const thanks = computed(() =>
  voted.value && voted.value.id === props.msg?.comfortId
    ? t(voted.value.verdict === 'mute' ? 'widget.comfort.muted' : 'widget.comfort.thanks')
    : null,
)

function vote(verdict: ComfortVerdict): void {
  const id = props.msg?.comfortId
  if (id == null) return
  voted.value = { id, verdict }
  emit('vote', id, verdict)
}

const VERDICTS: ComfortVerdict[] = ['useful', 'unfit', 'mute']
</script>

<template>
  <div class="area" :class="{ actionable: msg?.action }">
    <!-- 带动作的消息：整句是按钮，点了不算“单击小组件打开对话” -->
    <button
      v-if="msg?.action"
      class="message link"
      :title="msg.text"
      @pointerdown.stop
      @keydown.enter.stop
      @click.stop="emit('act', msg)"
    >
      {{ text }}
    </button>
    <p v-else class="message" :title="msg?.text" aria-live="polite">{{ text }}</p>
    <div class="meta">
      <div
        v-if="canVote"
        class="vote"
        role="group"
        :aria-label="t('widget.comfort.label')"
        @pointerdown.stop
        @keydown.enter.stop
      >
        <button
          v-for="v in VERDICTS"
          :key="v"
          class="compact"
          :data-verdict="v"
          :aria-label="t(`widget.comfort.${v}`)"
          :title="t(`widget.comfort.${v}`)"
          @click.stop="vote(v)"
        >
          {{ [...t(`widget.comfort.${v}`)][0] }}
        </button>
      </div>
      <span v-else-if="thanks" class="thanks" role="status">{{ thanks }}</span>
      <span v-if="msg?.ai" class="ai-tag">{{ t('common.ai_generated') }}</span>
    </div>
  </div>
</template>

<style scoped>
.area {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-height: 0;
}

.message {
  display: -webkit-box;
  margin: 0;
  overflow: hidden;
  font-size: var(--xq-fs-lg);
  font-weight: 500;
  line-height: var(--xq-lh-lg);
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2; /* FR-WGT-04：最多 2 行 */
}

/* 求助入口、周信提示：像链接一样能点，仍守 32 px 点击目标 */
.link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--xq-link);
  text-align: left;
  text-decoration: underline;
  text-underline-offset: 3px;
}

.meta {
  display: flex;
  flex: none;
  gap: var(--xq-sp-1);
  align-items: center;
  justify-content: flex-end;
  min-height: 32px;
  margin-top: auto;
}

/* 平时不显示，悬停或键盘聚焦一句话区时出现（FR-CMF-05）；用透明度而不是 visibility，键盘仍能 Tab 进来 */
.vote {
  display: flex;
  gap: var(--xq-sp-1);
  margin-right: auto;
  opacity: 0;
  pointer-events: none;
  transition: opacity var(--xq-dur-micro) var(--xq-ease);
}

.area:hover .vote,
.area:focus-within .vote {
  opacity: 1;
  pointer-events: auto;
}

.compact {
  padding: 0 var(--xq-sp-1);
}

.thanks {
  margin-right: auto;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

/* DS-COPY-05：AI 标识 */
.ai-tag {
  flex: none;
  padding: 0 var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
