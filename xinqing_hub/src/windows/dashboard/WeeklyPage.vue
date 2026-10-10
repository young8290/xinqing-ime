<script setup lang="ts">
// 看板 · 周报（07 FR-DSH-04）：一句话总结（本地模板，不调 AI）、休息完成率与饮水（和上周比）、新加的日程与完成的待办、
// 状态分布、“低落 / 疲劳”7 × 24 热力图、每日输入时长，以及作息洞察（RoutineInsight，FR-REV-03）。
// 每张图都有一句读屏概括，和一张“按天查看”的表格（dataviz：图表不只靠颜色，也不只靠悬停）。
// 专注时长（FR-RST-10）、晴天值（FR-REV-04）都是 P2，还没有数据，暂不显示。
import { computed, onMounted, ref, watch } from 'vue'
import { commands, unwrap, type WeekReport } from '@/api'
import EChart from '@/components/EChart.vue'
import type { ChartColors } from '@/components/chartTheme'
import { addDays, duration, localDate, shortDay, weekStart } from '@/format'
import { errorText, t } from '@/i18n'
import RoutineInsight from './RoutineInsight.vue'
import {
  distLabel,
  distOption,
  distRows,
  hasData,
  heatOption,
  typingOption,
  versus,
  weekRange,
  weekday,
} from './weekly'

const thisWeek = weekStart(localDate(new Date()))
const week = ref(thisWeek)
const report = ref<WeekReport | null>(null)
const last = ref<WeekReport | null>(null)
const error = ref<string | null>(null)

async function load(): Promise<void> {
  error.value = null
  try {
    const [r, l] = await Promise.all([
      unwrap(commands.weekStats(week.value)),
      unwrap(commands.weekStats(addDays(week.value, -7))),
    ])
    report.value = r
    last.value = l
  } catch (e) {
    error.value = errorText(e)
  }
}

onMounted(load)
watch(week, load)

const rows = computed(() => (report.value ? distRows(report.value.states) : []))
const lastData = computed(() => hasData(last.value))

const restRate = computed(() => report.value?.rest_rate ?? null)
const restVs = computed(() =>
  versus(restRate.value, lastData.value ? (last.value?.rest_rate ?? null) : null, (n) => `${n}%`),
)
const waterVs = computed(() =>
  report.value
    ? versus(report.value.water, lastData.value ? (last.value?.water ?? null) : null, (n) =>
        t('dashboard.weekly.water_value', { n }),
      )
    : '',
)

const distChart = computed(() => {
  const r = rows.value
  return (c: ChartColors) => distOption(r, c)
})
const heatChart = computed(() => {
  const r = report.value
  return (c: ChartColors) => heatOption(r?.heat ?? [], r?.days ?? [], c)
})
const typingChart = computed(() => {
  const r = report.value
  return (c: ChartColors) => typingOption(r?.days ?? [], c)
})

/** 热力图的一句话：最常出现在“周三 23 点”；没怎么出现就照实说 */
const heatLabel = computed(() => {
  const r = report.value
  if (!r) return ''
  let best: { d: number; h: number; n: number } | null = null
  r.heat.forEach((row, d) =>
    row.forEach((n, h) => {
      if (n > 0 && (!best || n > best.n)) best = { d, h, n }
    }),
  )
  if (!best) return t('dashboard.weekly.heat_none')
  const b = best as { d: number; h: number }
  const day = r.days[b.d]
  return t('dashboard.weekly.heat_label', {
    slot: `${day ? weekday(day.date) : ''} ${t('dashboard.weekly.heat_hour', { h: b.h })}`,
  })
})
const hardDayCounts = computed(() => (report.value?.heat ?? []).map((row) => row.reduce((s, n) => s + n, 0)))
const typingLabel = computed(() =>
  t('dashboard.weekly.typing_label', {
    parts: (report.value?.days ?? []).map((d) => `${weekday(d.date)} ${duration(d.typing_min)}`).join('，'),
  }),
)
</script>

<template>
  <section>
    <div class="d-head">
      <h1>{{ t('dashboard.nav.weekly') }}</h1>
      <div class="d-row">
        <button :aria-label="t('dashboard.weekly.prev')" @click="week = addDays(week, -7)">
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" class="chev">
            <path d="M10 3 5 8l5 5" />
          </svg>
        </button>
        <span class="range" aria-live="polite">{{ report ? weekRange(report) : '' }}</span>
        <button
          :aria-label="t('dashboard.weekly.next')"
          :disabled="week >= thisWeek"
          @click="week = addDays(week, 7)"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" class="chev">
            <path d="m6 3 5 5-5 5" />
          </svg>
        </button>
        <button v-if="week !== thisWeek" @click="week = thisWeek">
          {{ t('dashboard.weekly.this_week') }}
        </button>
      </div>
    </div>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>

    <template v-if="report">
      <section class="d-card line" :aria-label="t('dashboard.weekly.line')">
        <p>{{ report.line }}</p>
      </section>

      <ul class="tiles">
        <li class="d-card tile">
          <span class="tile-label">{{ t('dashboard.weekly.rest') }}</span>
          <span class="tile-value">{{ restRate === null ? t('dashboard.none') : `${restRate}%` }}</span>
          <span class="tile-detail">{{ restVs }}</span>
        </li>
        <li class="d-card tile">
          <span class="tile-label">{{ t('dashboard.weekly.water') }}</span>
          <span class="tile-value">{{ t('dashboard.weekly.water_value', { n: report.water }) }}</span>
          <span class="tile-detail">{{ waterVs }}</span>
        </li>
        <li class="d-card tile">
          <span class="tile-label">{{ t('dashboard.weekly.schedules') }}</span>
          <span class="tile-value">{{
            t('dashboard.weekly.count_value', { n: report.schedules_added })
          }}</span>
        </li>
        <li class="d-card tile">
          <span class="tile-label">{{ t('dashboard.weekly.todos') }}</span>
          <span class="tile-value">{{ t('dashboard.weekly.count_value', { n: report.todos_done }) }}</span>
        </li>
        <li v-if="report.focus_min > 0" class="d-card tile">
          <span class="tile-label">{{ t('dashboard.weekly.focus') }}</span>
          <span class="tile-value">{{ duration(report.focus_min) }}</span>
        </li>
      </ul>

      <div class="d-grid">
        <section class="d-card" aria-labelledby="dist-title">
          <h2 id="dist-title">{{ t('dashboard.weekly.dist') }}</h2>
          <p v-if="rows.length === 0" class="muted">{{ t('dashboard.weekly.dist_empty') }}</p>
          <div v-else class="plot" :style="{ height: `${rows.length * 32 + 16}px` }">
            <EChart :option="distChart" :label="distLabel(rows)" />
          </div>
        </section>

        <section class="d-card" aria-labelledby="typing-title">
          <h2 id="typing-title">{{ t('dashboard.weekly.typing') }}</h2>
          <div class="plot tall">
            <EChart :option="typingChart" :label="typingLabel" />
          </div>
        </section>

        <section class="d-card wide" aria-labelledby="heat-title">
          <h2 id="heat-title">{{ t('dashboard.weekly.heat') }}</h2>
          <p class="muted d-small">{{ t('dashboard.weekly.heat_hint') }}</p>
          <div class="plot heat">
            <EChart :option="heatChart" :label="heatLabel" />
          </div>
          <p class="d-small">{{ heatLabel }}</p>
        </section>

        <section class="d-card wide">
          <details>
            <summary>{{ t('dashboard.weekly.table') }}</summary>
            <table class="d-table">
              <thead>
                <tr>
                  <th scope="col">{{ t('dashboard.weekly.col_day') }}</th>
                  <th scope="col">{{ t('dashboard.weekly.col_typing') }}</th>
                  <th scope="col">{{ t('dashboard.weekly.col_rest') }}</th>
                  <th scope="col">{{ t('dashboard.weekly.col_water') }}</th>
                  <th scope="col">{{ t('dashboard.weekly.col_hard') }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(d, i) in report.days" :key="d.date">
                  <th scope="row">{{ weekday(d.date) }} {{ shortDay(d.date) }}</th>
                  <td>{{ duration(d.typing_min) }}</td>
                  <td>{{ t('dashboard.weekly.rest_cell', { done: d.rests_done, due: d.rests_due }) }}</td>
                  <td>{{ d.water }}</td>
                  <td>{{ hardDayCounts[i] ?? 0 }}</td>
                </tr>
              </tbody>
            </table>
          </details>
        </section>
      </div>
    </template>

    <div class="routine">
      <RoutineInsight />
    </div>
  </section>
</template>

<style scoped>
.range {
  min-width: 160px;
  font-weight: 500;
  text-align: center;
}

.line p {
  margin: 0;
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: var(--xq-sp-3);
  margin: var(--xq-sp-4) 0;
  padding: 0;
  list-style: none;
}

.tile {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-1);
}

.tile-label {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.tile-value {
  font-size: var(--xq-fs-xxl);
  font-weight: 600;
  line-height: var(--xq-lh-xxl);
}

.tile-detail {
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
}

/* 容器高度含坐标轴那一条，不出现卡片内的小滚动条（图表规范） */
.plot.tall {
  height: 220px;
}

.plot.heat {
  height: 240px;
}

.routine {
  margin-top: var(--xq-sp-4);
}

.muted {
  color: var(--xq-text-2);
}

.chev path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}
</style>
