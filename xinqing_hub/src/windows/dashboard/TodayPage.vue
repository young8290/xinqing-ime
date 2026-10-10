<script setup lang="ts">
// 看板 · 今日（07 FR-DSH-02）：四张概要卡片（输入时长、主导天气、暖心话次数、休息完成率）、打字心电图、
// 状态时间线（点开看解释，自评是实心圆点）、今天的自评、今日一句（暖心话列表与反馈）、写今天的日记。
// 统计由 day_stats 给（ADR 0036），每分钟刷新一次；自评只显示时间和选项，不显示备注。
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { commands, unwrap, type DayStats, type SelfReportItem } from '@/api'
import WeatherSprite from '@/components/WeatherSprite.vue'
import { clockOf, localDate } from '@/format'
import { errorText, t } from '@/i18n'
import { useStatusStore } from '@/stores/status'
import { WEATHER_ICON, hedge, weatherName } from '@/weather'
import ComfortList from './ComfortList.vue'
import StateTimeline from './StateTimeline.vue'
import TypingEcg from './TypingEcg.vue'
import { summaryCards } from './today'
import DiaryEditor from '../shared/DiaryEditor.vue'

const REFRESH_MS = 60_000

const status = useStatusStore()
const today = ref(localDate(new Date()))
const stats = ref<DayStats | null>(null)
const reports = ref<SelfReportItem[] | null>(null)
const error = ref<string | null>(null)
const writing = ref(false)
const saved = ref(false)
let timer: ReturnType<typeof setInterval> | undefined

const now = computed(() => {
  const s = status.snapshot
  return s && `${WEATHER_ICON[s.weather]} ${weatherName(s.weather)} · ${hedge(s.state)}`
})
const cards = computed(() => (stats.value ? summaryCards(stats.value) : []))
const newestFirst = computed(() => [...(reports.value ?? [])].sort((a, b) => b.ts - a.ts))

async function load(): Promise<void> {
  today.value = localDate(new Date())
  try {
    const [s, r] = await Promise.all([
      unwrap(commands.dayStats(today.value)),
      unwrap(commands.selfReportList(today.value)),
    ])
    stats.value = s
    reports.value = r
    error.value = null
  } catch (e) {
    error.value = errorText(e)
  }
}

onMounted(() => {
  status.init().catch((e) => console.warn('读取状态失败', e))
  void load()
  timer = setInterval(() => {
    if (!document.hidden) void load()
  }, REFRESH_MS)
})

onBeforeUnmount(() => clearInterval(timer))

function onSaved(): void {
  writing.value = false
  saved.value = true
}
</script>

<template>
  <section>
    <div class="d-head">
      <h1>{{ t('dashboard.nav.today') }}</h1>
      <button class="primary" :disabled="writing" @click="((writing = true), (saved = false))">
        {{ t('dashboard.today.write_diary') }}
      </button>
    </div>
    <p v-if="saved" class="muted" role="status">{{ t('diary.saved') }}</p>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>

    <section v-if="writing" class="d-card editor">
      <DiaryEditor @saved="onSaved" @cancel="writing = false" />
    </section>

    <ul v-if="cards.length" class="tiles">
      <li v-for="c in cards" :key="c.key" class="d-card tile">
        <span class="tile-label">{{ c.label }}</span>
        <span class="tile-value">{{ c.value }}</span>
        <span v-if="c.detail" class="tile-detail">{{ c.detail }}</span>
      </li>
    </ul>

    <div class="d-grid">
      <section v-if="status.snapshot" class="d-card now" :aria-label="t('dashboard.now')">
        <WeatherSprite :weather="status.snapshot.weather" :size="64" />
        <div>
          <h2>{{ t('dashboard.now') }}</h2>
          <p>{{ now }}</p>
        </div>
      </section>

      <section class="d-card" aria-labelledby="reports-title">
        <h2 id="reports-title">{{ t('dashboard.self_reports') }}</h2>
        <p v-if="reports && reports.length === 0" class="muted">{{ t('dashboard.self_reports_empty') }}</p>
        <ul v-else-if="reports" class="d-list reports">
          <li v-for="r in newestFirst" :key="r.id">
            <span class="d-time">{{ clockOf(r.ts) }}</span>
            {{ t(`self_report.options.${r.weather}`) }}
          </li>
        </ul>
      </section>

      <section class="d-card wide" aria-labelledby="ecg-title">
        <h2 id="ecg-title">{{ t('dashboard.today.ecg') }}</h2>
        <TypingEcg />
      </section>

      <section class="d-card wide" aria-labelledby="timeline-title">
        <h2 id="timeline-title">{{ t('dashboard.today.timeline') }}</h2>
        <p class="muted d-small">{{ t('dashboard.today.timeline_hint') }}</p>
        <StateTimeline v-if="stats" :date="today" :segments="stats.timeline" :reports="reports ?? []" />
      </section>

      <section class="d-card wide" aria-labelledby="comfort-title">
        <h2 id="comfort-title">{{ t('dashboard.today.comfort_title') }}</h2>
        <ComfortList :date="today" can-vote />
      </section>
    </div>
  </section>
</template>

<style scoped>
.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
  gap: var(--xq-sp-3);
  margin: 0 0 var(--xq-sp-4);
  padding: 0;
  list-style: none;
}

/* 概要卡片：标签、数字（看板数字用 --xq-fs-xxl，比例数字不用等宽）、补充说明 */
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

.editor {
  margin-bottom: var(--xq-sp-4);
}

.now {
  display: flex;
  gap: var(--xq-sp-4);
  align-items: center;
}

.now p {
  margin: 0;
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.reports li {
  padding: var(--xq-sp-1) 0;
}

.muted {
  color: var(--xq-text-2);
}
</style>
