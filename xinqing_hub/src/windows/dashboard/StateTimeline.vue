<script setup lang="ts">
// 状态时间线（07 FR-DSH-02、FR-DSH-03 点开某天）：0–24 时一条天气色带，自评是色带上方的实心圆点；
// 点任一段（或在“按时段查看”的列表里点）取那一刻的解释（state_explain(mood_id)，FR-STA-09）。
// 天气色只是填充：每段另有读屏名称，够宽时画图标，下方有图例和逐段列表，不只靠颜色区分（DS-COLOR-02）。
import { computed, ref, watch } from 'vue'
import { commands, unwrap, type Segment, type SelfReportItem } from '@/api'
import ExplainPanel from '@/components/ExplainPanel.vue'
import { explainLines, type ExplainLines } from '@/explain'
import { errorText, t } from '@/i18n'
import { WEATHER_ICON, weatherColor, weatherName } from '@/weather'
import { band, dayStart, dots, type BandSegment } from './today'

const props = defineProps<{
  date: string
  segments: readonly Segment[]
  reports: readonly SelfReportItem[]
}>()

const start = computed(() => dayStart(props.date))
const bands = computed(() => band(props.segments, start.value))
const marks = computed(() => dots(props.reports, start.value))
const legend = computed(() => [...new Set(bands.value.map((b) => b.weather).filter((w) => w !== null))])

const picked = ref<BandSegment | null>(null)
const lines = ref<ExplainLines | null>(null)
const missing = ref<string | null>(null)

watch(
  () => props.date,
  () => {
    picked.value = null
    lines.value = null
  },
)

async function pick(b: BandSegment): Promise<void> {
  picked.value = b
  lines.value = null
  missing.value = null
  try {
    const e = await unwrap(commands.stateExplain(b.seg.mood_id))
    if (e) lines.value = explainLines(e)
    else missing.value = t('dashboard.today.explain_none')
  } catch (err) {
    missing.value = errorText(err)
  }
}

/** 够宽（约 2 小时）才在色带上画图标 */
const WIDE = 8
const HOURS = [0, 6, 12, 18, 24]
</script>

<template>
  <div class="timeline">
    <p v-if="bands.length === 0" class="muted">{{ t('dashboard.today.timeline_empty') }}</p>
    <template v-else>
      <div class="track">
        <span
          v-for="m in marks"
          :key="m.id"
          class="dot"
          role="img"
          :style="{ left: `${m.left}%` }"
          :aria-label="m.label"
          :title="m.label"
        />
        <div class="band">
          <button
            v-for="b in bands"
            :key="b.seg.mood_id"
            class="seg"
            :class="{ active: picked === b }"
            :style="{
              left: `${b.left}%`,
              width: `${b.width}%`,
              background: b.weather ? weatherColor(b.weather) : undefined,
            }"
            :aria-label="b.label"
            :title="b.label"
            @click="pick(b)"
          >
            <span v-if="b.weather && b.width >= WIDE" aria-hidden="true">{{ WEATHER_ICON[b.weather] }}</span>
          </button>
        </div>
        <div class="hours" aria-hidden="true">
          <span v-for="h in HOURS" :key="h" :style="{ left: `${(h / 24) * 100}%` }">{{ h }}</span>
        </div>
      </div>
      <ul class="legend" :aria-label="t('dashboard.today.timeline')">
        <li v-for="w in legend" :key="w">
          <span class="swatch" :style="{ background: weatherColor(w) }" aria-hidden="true" />
          {{ WEATHER_ICON[w] }} {{ weatherName(w) }}
        </li>
      </ul>
      <section v-if="picked" class="explain" aria-live="polite">
        <h3>{{ t('dashboard.today.explain_title', { from: picked.from, to: picked.to }) }}</h3>
        <ExplainPanel v-if="lines" :lines="lines" />
        <p v-else-if="missing" class="muted">{{ missing }}</p>
      </section>
      <details>
        <summary>{{ t('dashboard.today.list') }}</summary>
        <ul class="segments">
          <li v-for="b in bands" :key="b.seg.mood_id">
            <button class="link" @click="pick(b)">{{ b.label }}</button>
          </li>
        </ul>
      </details>
    </template>
  </div>
</template>

<style scoped>
.track {
  position: relative;
  padding-top: 14px;
}

.band {
  position: relative;
  height: 32px;
  overflow: hidden;
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
}

/* 每段一个按钮：左边 2 px 底色的缝把相邻两段分开（图表规范的 surface gap） */
.seg {
  position: absolute;
  top: 0;
  bottom: 0;
  min-width: 2px;
  min-height: 0;
  padding: 0;
  border: none;
  border-left: 2px solid var(--xq-surface);
  border-radius: 0;
  background: var(--xq-text-3);
  color: var(--xq-on-weather);
  font-size: var(--xq-fs-sm);
}

.seg.active {
  outline: 2px solid var(--xq-focus);
  outline-offset: -2px;
}

/* 自评：实心圆点，带 2 px 底色外圈 */
.dot {
  position: absolute;
  top: 0;
  width: 10px;
  height: 10px;
  margin-left: -5px;
  border: 2px solid var(--xq-surface);
  border-radius: 50%;
  background: var(--xq-text-1);
  box-sizing: content-box;
}

.hours {
  position: relative;
  height: 18px;
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.hours span {
  position: absolute;
  transform: translateX(-50%);
}

.hours span:first-child {
  transform: none;
}

.hours span:last-child {
  transform: translateX(-100%);
}

.legend {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  margin: var(--xq-sp-2) 0 0;
  padding: 0;
  list-style: none;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.swatch {
  display: inline-block;
  width: 12px;
  height: 12px;
  border-radius: 3px;
  vertical-align: -1px;
}

.explain {
  margin-top: var(--xq-sp-3);
  padding: var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
}

.explain h3 {
  margin: 0 0 var(--xq-sp-1);
  font-size: var(--xq-fs-sm);
  font-weight: 500;
}

.segments {
  margin: 0;
  padding-left: var(--xq-sp-4);
  font-size: var(--xq-fs-sm);
}

.link {
  min-height: 28px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--xq-link);
  text-align: left;
}

.muted {
  color: var(--xq-text-2);
}
</style>
