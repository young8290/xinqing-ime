<script setup lang="ts">
// 休息提醒卡片（06 FR-RST-06 第 4 条）：一句完整文案和 `已完成` / `5 分钟后` / `今天不再提醒`。
// 盖住小组件的整个内容区，与自评面板同一位置；Esc 等同“5 分钟后”，不清零也不关闭当天提醒。
import { onMounted, ref } from 'vue'
import type { RestAction, RestDue } from '@/api'
import { t } from '@/i18n'
import { doneLabel, restText } from './useRest'

const props = defineProps<{ due: RestDue; countdown: number | null }>()
const emit = defineEmits<{ act: [action: RestAction] }>()

const ICONS = { eye: '👀', water: '💧', move: '🚶', night: '🌙' } as const
const root = ref<HTMLElement | null>(null)

onMounted(() => root.value?.querySelector<HTMLButtonElement>('button')?.focus())
</script>

<template>
  <div
    ref="root"
    class="rest"
    role="alertdialog"
    :aria-label="t('notify.rest_title')"
    @pointerdown.stop
    @keydown.esc.stop="emit('act', 'later')"
    @keydown.enter.stop
  >
    <p class="text">
      <span aria-hidden="true">{{ ICONS[props.due.kind] }}</span>
      {{ t(restText(props.due.kind, props.due.tired)) }}
    </p>
    <p v-if="props.countdown !== null" class="countdown" role="timer" aria-live="polite">
      {{ props.countdown > 0 ? props.countdown : t('rest.eye_done') }}
    </p>
    <div v-else class="actions">
      <button class="compact primary" @click.stop="emit('act', 'done')">
        {{ t(doneLabel(props.due.kind)) }}
      </button>
      <button class="compact" @click.stop="emit('act', 'later')">{{ t('rest.btn_later') }}</button>
      <button class="compact" @click.stop="emit('act', 'today_off')">{{ t('rest.btn_today_off') }}</button>
    </div>
  </div>
</template>

<style scoped>
.rest {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-2);
  justify-content: center;
}

.text {
  margin: 0;
  font-size: var(--xq-fs-lg);
  font-weight: 500;
  line-height: var(--xq-lh-lg);
}

.countdown {
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
  font-variant-numeric: tabular-nums;
}

.actions {
  display: flex;
  gap: var(--xq-sp-1);
}

/* 仍守 32 × 32 的点击目标（DS-A11Y-03），只收窄左右留白 */
.compact {
  padding: 0 var(--xq-sp-2);
  font-size: var(--xq-fs-xs);
  white-space: nowrap;
}
</style>
