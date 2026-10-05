<script setup lang="ts">
// “陪我呼吸 1 分钟”（05 FR-CHT-06）：吸 4 秒、停 4 秒、呼 6 秒，共 4 轮。纯本地动画，不出网、不写库。
// 系统关闭动画时（DS-MOTION-03）圆圈不缩放，文字照常按节奏切换。
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { t } from '@/i18n'
import { PHASES, ROUNDS } from './breathing'

const emit = defineEmits<{ close: [] }>()

const round = ref(1)
const step = ref(0)
const finished = ref(false)
let timer: ReturnType<typeof setTimeout> | undefined
const closeBtn = ref<HTMLButtonElement | null>(null)

const phase = computed(() => PHASES[step.value]!)
const label = computed(() => {
  if (finished.value) return t('chat.breathe_done')
  return t(
    phase.value.key === 'in'
      ? 'chat.breathe_in'
      : phase.value.key === 'hold'
        ? 'chat.breathe_hold'
        : 'chat.breathe_out',
  )
})

function schedule(): void {
  timer = setTimeout(() => {
    if (step.value < PHASES.length - 1) step.value += 1
    else if (round.value < ROUNDS) {
      round.value += 1
      step.value = 0
    } else {
      finished.value = true
      return
    }
    schedule()
  }, phase.value.seconds * 1000)
}

onMounted(() => {
  schedule()
  // 盖住整个窗口的对话框：焦点移到“结束”按钮，Esc 由对话窗口关掉它（DS-A11Y-01）
  closeBtn.value?.focus()
})
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <div class="breathing" role="dialog" aria-modal="true" :aria-label="t('chat.shortcut_breathe')">
    <div
      class="circle"
      :class="finished ? 'rest' : phase.key"
      :style="{ transitionDuration: `${phase.seconds}s` }"
      aria-hidden="true"
    />
    <p class="label" aria-live="polite">{{ label }}</p>
    <p v-if="!finished" class="round">{{ t('chat.breathe_round', { n: round }) }}</p>
    <button ref="closeBtn" class="compact" @click="emit('close')">{{ t('chat.breathe_close') }}</button>
  </div>
</template>

<style scoped>
.breathing {
  position: absolute;
  inset: 0;
  z-index: 3;
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-3);
  align-items: center;
  justify-content: center;
  background: var(--xq-surface);
}

.circle {
  width: 120px;
  height: 120px;
  border-radius: 50%;
  background: var(--xq-primary);
  opacity: 0.6;
  transform: scale(0.6);
  transition-property: transform;
  transition-timing-function: ease-in-out;
}

.circle.in,
.circle.hold {
  transform: scale(1);
}

.label {
  margin: 0;
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.round {
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.compact {
  padding: 0 var(--xq-sp-3);
  font-size: var(--xq-fs-sm);
}
</style>
