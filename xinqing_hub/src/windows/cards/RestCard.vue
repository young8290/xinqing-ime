<script setup lang="ts">
// 休息提醒卡片（06 FR-RST-06 第 4 条）：一句完整文案和 `已完成` / `5 分钟后` / `今天不再提醒`。
// Esc 等同“5 分钟后”，不清零也不关闭当天提醒。护眼点“已完成”先在卡片上倒数 20 秒，再显示“眼睛说谢谢你”2 秒，
// 之后才交给后端（FR-RST-02）；倒计时按秒变数字，不做闪烁（DS-MOTION-03/04）。
import { onBeforeUnmount, ref } from 'vue'
import type { RestAction, RestDue } from '@/api'
import { t } from '@/i18n'
import CardShell from './CardShell.vue'
import { EYE_SECONDS, THANKS_MS, doneLabel, restText } from './rest'

const props = defineProps<{ due: RestDue; error?: string | null }>()
const emit = defineEmits<{ act: [action: RestAction] }>()

const ICONS = { eye: '👀', water: '💧', move: '🚶', night: '🌙' } as const
/** 护眼倒计时剩余秒数；没在倒计时为 null，0 表示倒计时结束、正在显示致谢 */
const countdown = ref<number | null>(null)
let timer: ReturnType<typeof setInterval> | undefined
let thanks: ReturnType<typeof setTimeout> | undefined

function act(action: RestAction): void {
  if (countdown.value !== null) return
  if (action === 'done' && props.due.kind === 'eye') {
    countdown.value = EYE_SECONDS
    timer = setInterval(() => {
      countdown.value = (countdown.value ?? 1) - 1
      if (countdown.value > 0) return
      clearInterval(timer)
      thanks = setTimeout(() => emit('act', 'done'), THANKS_MS)
    }, 1000)
    return
  }
  emit('act', action)
}

onBeforeUnmount(() => {
  clearInterval(timer)
  clearTimeout(thanks)
})
</script>

<template>
  <CardShell
    class="rest"
    :label="t('notify.rest_title')"
    :error="props.error"
    @primary="act('done')"
    @dismiss="act('later')"
  >
    <template #title>
      <span aria-hidden="true">{{ ICONS[props.due.kind] }}</span>
      {{ t(restText(props.due.kind, props.due.tired)) }}
    </template>
    <p v-if="countdown !== null" class="countdown" role="timer" aria-live="polite">
      {{ countdown > 0 ? countdown : t('rest.eye_done') }}
    </p>
    <template v-if="countdown === null" #actions>
      <button class="primary" @click="act('done')">{{ t(doneLabel(props.due.kind)) }}</button>
      <button @click="act('later')">{{ t('rest.btn_later') }}</button>
      <button @click="act('today_off')">{{ t('rest.btn_today_off') }}</button>
    </template>
  </CardShell>
</template>

<style scoped>
.countdown {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
  font-variant-numeric: tabular-nums;
}
</style>
