<script setup lang="ts">
// 到点提醒与错过的提醒（06 FR-SCH-07、FR-SCH-14）：标题与正文都是后端按 [notify] 文案拼好的（锁屏可见，不含情绪词，
// DS-COPY-08）。到点提醒：知道了 / 5 分钟后 / 10 分钟后（每日汇总只有“知道了”）；错过的提醒列出每一条，可以去看日程。
// Esc 或 × 收起到点提醒等同“5 分钟后”（与休息提醒卡片一致：没确认过的提醒不丢）。
import { computed } from 'vue'
import type { ReminderDue, ReminderMissed } from '@/api'
import { t } from '@/i18n'
import CardShell from './CardShell.vue'

const props = defineProps<{ due?: ReminderDue; missed?: ReminderMissed; error?: string | null }>()
const emit = defineEmits<{ act: [action: string]; open: []; dismiss: [] }>()

const head = computed(() => props.due ?? props.missed!)
const BELL = '🔔'
const snooze = computed(() => props.due !== undefined && props.due.kind !== 'todo_digest')
</script>

<template>
  <CardShell
    :label="head.title"
    :error="props.error"
    @primary="props.due ? emit('act', 'ok') : emit('dismiss')"
    @dismiss="props.due ? emit('act', snooze ? 'snooze_5' : 'ok') : emit('dismiss')"
  >
    <template #title>
      <span aria-hidden="true">{{ BELL }}</span> {{ head.title }}
    </template>
    <p>{{ head.body }}</p>
    <ul v-if="props.missed" class="items">
      <li v-for="i in props.missed.items" :key="i.id">{{ i.title }} · {{ i.body }}</li>
    </ul>
    <template #actions>
      <template v-if="props.due">
        <button class="primary" @click="emit('act', 'ok')">{{ t('notify.btn_ok') }}</button>
        <template v-if="snooze">
          <button @click="emit('act', 'snooze_5')">{{ t('notify.btn_snooze') }}</button>
          <button @click="emit('act', 'snooze_10')">{{ t('notify.btn_snooze10') }}</button>
        </template>
      </template>
      <template v-else>
        <button class="primary" @click="emit('dismiss')">{{ t('notify.btn_ok') }}</button>
        <button @click="emit('open')">{{ t('cards.missed_open') }}</button>
      </template>
    </template>
  </CardShell>
</template>

<style scoped>
.items {
  max-height: 72px;
  margin: var(--xq-sp-1) 0 0;
  padding-left: var(--xq-sp-4);
  overflow-y: auto;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
