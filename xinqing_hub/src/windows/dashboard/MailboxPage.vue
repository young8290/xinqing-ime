<script setup lang="ts">
// 看板 · 信箱（07 FR-DSH-06、05 FR-REV-02）：晴晴的周信按时间倒序，未读的有小圆点和“未读”；
// 打开后以信纸样式显示（暖色底），AI 写的信末标 `AI 生成`，打开即标为已读；可以删除（删掉的不会重写）。
// 从卡片层“打开看看”或一句话区的周信提示进来时，`letter` 指明直接打开哪一封。
import { computed, onMounted, ref, watch } from 'vue'
import { commands, unwrap, type LetterItem } from '@/api'
import { dayLabel } from '@/format'
import { errorText, t } from '@/i18n'

const props = defineProps<{ letter?: number }>()

const letters = ref<LetterItem[] | null>(null)
const openId = ref<number | null>(null)
const confirming = ref(false)
const error = ref<string | null>(null)

const current = computed(() => letters.value?.find((l) => l.id === openId.value) ?? null)

async function load(): Promise<void> {
  try {
    letters.value = await unwrap(commands.lettersList())
    error.value = null
  } catch (e) {
    error.value = errorText(e)
  }
}

async function open(id: number): Promise<void> {
  openId.value = id
  confirming.value = false
  const l = letters.value?.find((x) => x.id === id)
  if (l && !l.read) {
    try {
      await unwrap(commands.letterRead(id))
      l.read = true
    } catch (e) {
      error.value = errorText(e)
    }
  }
}

async function remove(id: number): Promise<void> {
  try {
    await unwrap(commands.letterDelete(id))
    openId.value = null
    confirming.value = false
    await load()
  } catch (e) {
    error.value = errorText(e)
  }
}

onMounted(async () => {
  await load()
  if (props.letter) await open(props.letter)
})
watch(
  () => props.letter,
  (id) => id && open(id),
)
</script>

<template>
  <section>
    <h1>{{ t('dashboard.nav.mailbox') }}</h1>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>

    <article
      v-if="current"
      class="paper"
      :aria-label="t('dashboard.mailbox.week_of', { date: dayLabel(current.week_start) })"
    >
      <button class="back" @click="openId = null">{{ t('dashboard.mailbox.back') }}</button>
      <h2 class="paper-title">
        {{ t('dashboard.mailbox.week_of', { date: dayLabel(current.week_start) }) }}
      </h2>
      <p class="body">{{ current.content }}</p>
      <p v-if="current.ai_generated" class="sign">
        <span class="d-tag">{{ t('common.ai_generated') }}</span>
      </p>
      <div class="d-row">
        <template v-if="confirming">
          <span class="d-small">{{ t('dashboard.mailbox.delete_confirm') }}</span>
          <button class="danger" @click="remove(current.id)">{{ t('dashboard.mailbox.confirm') }}</button>
          <button @click="confirming = false">{{ t('dashboard.mailbox.cancel') }}</button>
        </template>
        <button v-else @click="confirming = true">{{ t('dashboard.mailbox.delete') }}</button>
      </div>
    </article>

    <template v-else-if="letters">
      <p v-if="letters.length === 0" class="muted">{{ t('dashboard.mailbox.empty') }}</p>
      <ul v-else class="d-list letters">
        <li v-for="l in letters" :key="l.id">
          <button class="row" @click="open(l.id)">
            <span v-if="!l.read" class="unread-dot" aria-hidden="true" />
            <span class="week">{{ t('dashboard.mailbox.week_of', { date: dayLabel(l.week_start) }) }}</span>
            <span v-if="!l.read" class="unread">{{ t('dashboard.mailbox.unread') }}</span>
            <span v-if="l.ai_generated" class="d-tag">{{ t('common.ai_generated') }}</span>
          </button>
        </li>
      </ul>
      <p class="muted d-small">{{ t('dashboard.mailbox.hint') }}</p>
    </template>
  </section>
</template>

<style scoped>
.row {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  width: 100%;
  border-color: transparent;
  background: transparent;
  text-align: left;
}

.row:hover {
  background: var(--xq-surface-2);
}

.week {
  flex: 1;
}

/* 未读：小圆点 + 文字，不只靠颜色（DS-COLOR-02） */
.unread-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--xq-link);
}

.unread {
  color: var(--xq-link);
  font-size: var(--xq-fs-sm);
}

/* 信纸：暖色底（晴天色淡淡地铺一层，文字仍用主文字色，对比度不变）、标题字距放宽一点 */
.paper {
  max-width: 640px;
  padding: var(--xq-sp-5) var(--xq-sp-6);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
  background: color-mix(in srgb, var(--xq-w-sunny) 12%, var(--xq-surface));
  box-shadow: var(--xq-shadow-card);
}

.paper-title {
  margin: var(--xq-sp-3) 0;
  font-size: var(--xq-fs-xl);
  letter-spacing: 2px;
}

.body {
  font-size: var(--xq-fs-lg);
  line-height: 1.9;
  white-space: pre-wrap;
}

.sign {
  text-align: right;
}

.back {
  border-color: transparent;
  background: transparent;
  color: var(--xq-link);
}

.danger {
  border-color: var(--xq-danger);
  color: var(--xq-danger);
}

.muted {
  color: var(--xq-text-2);
}

@media (forced-colors: active) {
  .paper {
    background: Canvas;
  }
}
</style>
