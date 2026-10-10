<script setup lang="ts">
// 日程卡片与待办卡片（06 FR-SCH-05、FR-SCH-11、FR-SCH-13）：先“识别中…”，抽取完成后填字段，带 `AI 识别`；
// 日程有时间重叠时多一行“⚠ 与「…」时间重叠”；按钮 添加 / 修改 / 忽略（待办是 加入待办 / 修改 / 忽略）。
import { computed } from 'vue'
import { t } from '@/i18n'
import CardShell from './CardShell.vue'
import { conflictText, flagTexts, scheduleWhen, todoDue } from './cards'
import type { Card } from './queue'

type Agenda = Extract<Card, { kind: 'schedule' | 'todo' }>

const props = defineProps<{ card: Agenda; error?: string | null }>()
const emit = defineEmits<{ add: []; edit: []; ignore: []; dismiss: [] }>()

const isSchedule = computed(() => props.card.kind === 'schedule')
const title = computed(() => props.card.item?.title ?? '')
const lines = computed(() => {
  const c = props.card
  if (!c.item) return []
  if (c.kind === 'schedule') {
    const out = [scheduleWhen(c.item)]
    if (c.item.location) out.push(t('cards.location', { location: c.item.location }))
    return out
  }
  return [todoDue(c.item)].filter((x): x is string => x !== null)
})
const notes = computed(() => {
  const c = props.card
  if (c.kind !== 'schedule' || !c.item) return []
  return [...flagTexts(c.item.flags), ...c.conflicts.map(conflictText)]
})
const label = computed(() => t(isSchedule.value ? 'cards.schedule_label' : 'cards.todo_label'))
</script>

<template>
  <CardShell
    :label="title ? `${label}：${title}` : label"
    :error="props.error"
    :tag="props.card.item && props.card.item.source === 'ai' ? t('common.ai_detected') : null"
    @primary="props.card.item && !props.card.done && emit('add')"
    @dismiss="emit('dismiss')"
  >
    <template #title>
      <template v-if="!props.card.item">
        {{ isSchedule ? t('cards.detecting_schedule') : t('cards.detecting_todo') }}
      </template>
      <template v-else>{{ isSchedule ? '📅' : '✅' }} {{ title }}</template>
    </template>
    <template v-if="props.card.item">
      <p v-for="l in lines" :key="l">{{ l }}</p>
      <p v-for="n in notes" :key="n" class="muted note">{{ n }}</p>
    </template>
    <template v-if="props.card.item" #actions>
      <p v-if="props.card.done" class="done" role="status">
        {{ isSchedule ? t('cards.added') : t('cards.todo_added') }}
      </p>
      <template v-else>
        <button class="primary" @click="emit('add')">
          {{ isSchedule ? t('cards.add') : t('cards.todo_add') }}
        </button>
        <button @click="emit('edit')">{{ t('cards.edit') }}</button>
        <button @click="emit('ignore')">{{ t('cards.ignore') }}</button>
      </template>
    </template>
  </CardShell>
</template>

<style scoped>
.note {
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.done {
  margin: 0;
  color: var(--xq-text-2);
  line-height: 32px;
}
</style>
