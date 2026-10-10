<script setup lang="ts">
// 打字心电图（07 FR-DSH-02）：最近 60 秒的键间间隔，实时滚动；长停顿是尖峰，成串退格是红色三角，状态变化处一条竖线带天气图标。
// 数据来自 typing:pulse（只有间隔与是不是退格，不含键值和文字，后端只在看板看得见时推送，ADR 0036）；
// 刷新不超过 10 Hz，窗口看不见时不重画（DS-MOTION-05）。图下一句话概括与图例，读屏和色觉障碍都读得到。
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { events, type Pulse, type Weather } from '@/api'
import EChart from '@/components/EChart.vue'
import { t } from '@/i18n'
import { ECG_WINDOW_MS, ecgLabel, ecgOption, trim } from './today'

/** 重画间隔：5 Hz，低于规定的 10 Hz 上限 */
const TICK_MS = 200

const pulses = ref<Pulse[]>([])
const changes = ref<{ ts: number; weather: Weather }[]>([])
const now = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
const unlisten: UnlistenFn[] = []

const shown = computed(() => trim(pulses.value, now.value))
const option = computed(() => {
  const p = shown.value
  const c = changes.value.filter((x) => x.ts > now.value - ECG_WINDOW_MS)
  const at = now.value
  return (colors: Parameters<typeof ecgOption>[3]) => ecgOption(p, c, at, colors)
})
const label = computed(() => ecgLabel(shown.value))

onMounted(async () => {
  timer = setInterval(() => {
    if (document.hidden) return
    now.value = Date.now()
    // 只留窗口内的，不让数组越攒越长
    if (pulses.value.length > 0 && pulses.value[0]!.ts <= now.value - ECG_WINDOW_MS)
      pulses.value = shown.value
  }, TICK_MS)
  try {
    let last: Weather | null = null
    unlisten.push(
      await events.typingPulse.listen((e) => {
        pulses.value = [...pulses.value, ...e.payload.keys]
      }),
      await events.statusChanged.listen((e) => {
        const w = e.payload.weather
        if (last !== null && w !== last) changes.value = [...changes.value, { ts: Date.now(), weather: w }]
        last = w
      }),
    )
  } catch (e) {
    console.warn('订阅打字心电图失败', e)
  }
})

onBeforeUnmount(() => {
  clearInterval(timer)
  for (const u of unlisten) u()
})
</script>

<template>
  <div class="ecg">
    <div class="plot">
      <EChart :option="option" :label="label" />
    </div>
    <p class="caption">{{ shown.length === 0 ? t('dashboard.today.ecg_idle') : label }}</p>
    <ul class="keys">
      <li><span class="key-line" aria-hidden="true" />{{ t('dashboard.today.ecg_key_iki') }}</li>
      <li><span class="key-tri" aria-hidden="true" />{{ t('dashboard.today.ecg_key_bs') }}</li>
      <li><span class="key-rule" aria-hidden="true" />{{ t('dashboard.today.ecg_key_state') }}</li>
    </ul>
    <p class="hint">{{ t('dashboard.today.ecg_hint') }}</p>
  </div>
</template>

<style scoped>
.plot {
  height: 160px;
}

.caption {
  margin: var(--xq-sp-1) 0 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.keys {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  margin: var(--xq-sp-1) 0 0;
  padding: 0;
  list-style: none;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
}

.keys li {
  display: flex;
  gap: var(--xq-sp-1);
  align-items: center;
}

.key-line {
  width: 16px;
  height: 2px;
  border-radius: 1px;
  background: var(--xq-chart-1);
}

.key-tri {
  width: 0;
  height: 0;
  border-right: 5px solid transparent;
  border-bottom: 9px solid var(--xq-danger);
  border-left: 5px solid transparent;
}

.key-rule {
  width: 1px;
  height: 12px;
  background: var(--xq-text-3);
}

.hint {
  margin: var(--xq-sp-1) 0 0;
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
