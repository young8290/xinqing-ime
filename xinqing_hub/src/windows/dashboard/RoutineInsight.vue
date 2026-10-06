<script setup lang="ts">
// 周报 · 作息洞察（05 FR-REV-03、07 FR-DSH-04）：最近 7 / 30 天的停止打字时间折线、平均停止时间、晚于零点的晚数。
// 一律称“停止打字时间”，并注明不等于入睡时间。图下有“按天查看”表格，数值不只靠图（dataviz：表格视图）。
import { computed, onMounted, ref } from 'vue'
import { commands, unwrap, type Routine } from '@/api'
import EChart from '@/components/EChart.vue'
import { errorText, t } from '@/i18n'
import { routineLabel, routineOption, shortDate, stopTime } from './routine'

const RANGES = [7, 30] as const
type Days = (typeof RANGES)[number]

const days = ref<Days>(7)
const routine = ref<Routine | null>(null)
const error = ref<string | null>(null)
const loading = ref(false)

const option = computed(() => {
  const r = routine.value
  return r ? (c: Parameters<typeof routineOption>[1]) => routineOption(r, c) : null
})

async function load(d: Days): Promise<void> {
  days.value = d
  error.value = null
  loading.value = true
  try {
    routine.value = await unwrap(commands.getRoutine(d))
  } catch (e) {
    error.value = errorText(e)
  } finally {
    loading.value = false
  }
}

onMounted(() => load(7))
</script>

<template>
  <section class="card" aria-labelledby="routine-title">
    <header class="head">
      <h2 id="routine-title">{{ t('dashboard.routine.title') }}</h2>
      <div class="seg" role="group" :aria-label="t('dashboard.routine.title')">
        <button
          v-for="d in RANGES"
          :key="d"
          class="compact"
          :aria-pressed="days === d"
          :disabled="loading"
          @click="load(d)"
        >
          {{ t(`dashboard.routine.range_${d}`) }}
        </button>
      </div>
    </header>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <template v-else-if="routine">
      <dl class="stats">
        <div>
          <dt>{{ t('dashboard.routine.avg') }}</dt>
          <dd>
            {{
              routine.avg_stop_min === null ? t('dashboard.routine.no_avg') : stopTime(routine.avg_stop_min)
            }}
          </dd>
        </div>
        <div>
          <dt>{{ t('dashboard.routine.late') }}</dt>
          <dd>{{ t('dashboard.routine.late_value', { n: routine.late_nights }) }}</dd>
        </div>
        <div>
          <dt>{{ t('dashboard.routine.counted') }}</dt>
          <dd>
            {{
              t('dashboard.routine.counted_value', {
                n: routine.counted_nights,
                total: routine.nights.length,
              })
            }}
          </dd>
        </div>
      </dl>

      <p v-if="routine.counted_nights === 0" class="muted">{{ t('dashboard.routine.none') }}</p>
      <div v-else-if="option" class="plot">
        <EChart :option="option" :label="routineLabel(routine)" />
      </div>

      <p class="note">{{ t('routine.stop_typing_note') }}</p>

      <details v-if="routine.counted_nights > 0" class="table">
        <summary>{{ t('dashboard.routine.table') }}</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">{{ t('dashboard.routine.col_night') }}</th>
              <th scope="col">{{ t('dashboard.routine.col_stop') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="n in [...routine.nights].reverse()" :key="n.date">
              <td>{{ shortDate(n.date) }}</td>
              <td :class="{ muted: n.stop_min === null }">
                {{ n.stop_min === null ? t('dashboard.routine.no_record') : stopTime(n.stop_min) }}
                <span v-if="n.late" class="tag">{{ t('dashboard.routine.late') }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </details>
    </template>
  </section>
</template>

<style scoped>
.card {
  padding: var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-card);
}

.head {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  align-items: center;
  justify-content: space-between;
}

.head h2 {
  margin: 0;
}

.seg {
  display: flex;
  gap: var(--xq-sp-1);
}

.seg button {
  padding: 0 var(--xq-sp-3);
}

.seg button[aria-pressed='true'] {
  border-color: var(--xq-text-2);
  background: var(--xq-surface-2);
  font-weight: 600;
}

.stats {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-5);
  margin: var(--xq-sp-4) 0 var(--xq-sp-2);
}

.stats div {
  min-width: 120px;
}

.stats dt {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.stats dd {
  margin: var(--xq-sp-1) 0 0;
  font-size: var(--xq-fs-xl);
  line-height: var(--xq-lh-xl);
  font-weight: 600;
}

.plot {
  height: 240px;
}

.note {
  margin: var(--xq-sp-2) 0 0;
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
}

.table {
  margin-top: var(--xq-sp-3);
  font-size: var(--xq-fs-sm);
}

.table summary {
  min-height: 32px;
  line-height: 32px;
  color: var(--xq-text-2);
  cursor: pointer;
}

.table table {
  border-collapse: collapse;
  font-variant-numeric: tabular-nums;
}

.table th,
.table td {
  padding: var(--xq-sp-1) var(--xq-sp-4) var(--xq-sp-1) 0;
  border-bottom: 1px solid var(--xq-border);
  text-align: left;
}

.tag {
  margin-left: var(--xq-sp-2);
  padding: 0 var(--xq-sp-1);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
}

.error {
  color: var(--xq-danger);
}
</style>
