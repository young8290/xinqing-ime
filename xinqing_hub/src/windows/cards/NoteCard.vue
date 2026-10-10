<script setup lang="ts">
// 三种简单卡片（07 FR-WGT-07）：
// - 自评回应：晴晴的回应在小组件一句话区，这里给“和晴晴聊聊”（04 FR-STA-10 第 2 条）；
// - 晚间小结：统计、天气色带、一句本地模板（不加 AI 标识），“看看今天的看板”/“今天不用了”（05 FR-REV-01）；
// - 周信提示：“✉️ 晴晴给你写了一封信”，打开看看 / 等会儿再看（05 FR-REV-02，不发系统通知）。
import { computed } from 'vue'
import { t } from '@/i18n'
import CardShell from './CardShell.vue'
import { eveningBand, eveningLines } from './cards'
import type { Card } from './queue'

type Note = Extract<Card, { kind: 'self_reply' | 'evening' | 'letter' }>

const props = defineProps<{ card: Note; error?: string | null }>()
const emit = defineEmits<{ primary: []; secondary: []; dismiss: [] }>()

const title = computed(() => {
  switch (props.card.kind) {
    case 'self_reply':
      return t('cards.self_reply_title')
    case 'evening':
      return t('cards.evening_title')
    default:
      return t('cards.letter_title')
  }
})

const buttons = computed((): [string, string] => {
  switch (props.card.kind) {
    case 'self_reply':
      return [t('cards.self_reply_chat'), t('cards.self_reply_later')]
    case 'evening':
      return [t('cards.evening_open'), t('cards.evening_dismiss')]
    default:
      return [t('cards.letter_open'), t('cards.letter_later')]
  }
})

const evening = computed(() => (props.card.kind === 'evening' ? props.card.summary : null))
const band = computed(() => (evening.value ? eveningBand(evening.value) : null))
</script>

<template>
  <CardShell
    :label="title"
    :error="props.error"
    :tag="props.card.kind === 'letter' && props.card.letter.ai_generated ? t('common.ai_generated') : null"
    @primary="emit('primary')"
    @dismiss="emit('dismiss')"
  >
    <template #title>{{ title }}</template>
    <template v-if="evening">
      <p v-for="(row, i) in eveningLines(evening)" :key="i" class="stats">
        <span v-for="item in row" :key="item">{{ item }}</span>
      </p>
      <p v-if="band && band.icons" class="band" role="img" :aria-label="band.label">{{ band.icons }}</p>
      <p>{{ evening.line }}</p>
    </template>
    <template #actions>
      <button class="primary" @click="emit('primary')">{{ buttons[0] }}</button>
      <button @click="emit('secondary')">{{ buttons[1] }}</button>
    </template>
  </CardShell>
</template>

<style scoped>
.stats {
  display: flex;
  flex-wrap: wrap;
  column-gap: var(--xq-sp-3);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.band {
  letter-spacing: 2px;
}
</style>
