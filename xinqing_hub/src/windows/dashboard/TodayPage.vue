<script setup lang="ts">
// 看板 · 今日（07 FR-DSH-02）。现在能做的：当前天气，今天的自评（只显示时间和选项，不显示备注——看板不显示用户输入的原文，FR-DSH-01）。
// 概要卡片、打字心电图、状态时间线、今天的暖心话要等对应的统计命令和事件（B / C），先说明还在准备中。
import { computed, onMounted, ref } from 'vue'
import { commands, unwrap, type SelfReportItem } from '@/api'
import WeatherSprite from '@/components/WeatherSprite.vue'
import { errorText, t } from '@/i18n'
import { localDate } from '@/selfReport'
import { useStatusStore } from '@/stores/status'
import { WEATHER_ICON, hedge, weatherName } from '@/weather'

const status = useStatusStore()
const reports = ref<SelfReportItem[] | null>(null)
const error = ref<string | null>(null)

const now = computed(() => {
  const s = status.snapshot
  return s && `${WEATHER_ICON[s.weather]} ${weatherName(s.weather)} · ${hedge(s.state)}`
})

const time = (ts: number) =>
  new Date(ts).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false })

onMounted(async () => {
  status.init().catch((e) => console.warn('读取状态失败', e))
  try {
    const list = await unwrap(commands.selfReportList(localDate(new Date())))
    reports.value = [...list].sort((a, b) => b.ts - a.ts)
  } catch (e) {
    error.value = errorText(e)
  }
})
</script>

<template>
  <section>
    <h1>{{ t('dashboard.today') }}</h1>

    <div class="grid">
      <section v-if="status.snapshot" class="card now" :aria-label="t('dashboard.now')">
        <WeatherSprite :weather="status.snapshot.weather" :size="64" />
        <div>
          <h2>{{ t('dashboard.now') }}</h2>
          <p>{{ now }}</p>
        </div>
      </section>

      <section class="card" aria-labelledby="reports-title">
        <h2 id="reports-title">{{ t('dashboard.self_reports') }}</h2>
        <p v-if="error" class="error" role="alert">{{ error }}</p>
        <p v-else-if="reports && reports.length === 0" class="muted">
          {{ t('dashboard.self_reports_empty') }}
        </p>
        <ul v-else-if="reports" class="reports">
          <li v-for="r in reports" :key="r.id">
            <span class="time">{{ time(r.ts) }}</span>
            {{ t(`self_report.options.${r.weather}`) }}
          </li>
        </ul>
      </section>
    </div>

    <p class="muted pending">{{ t('dashboard.today_pending') }}</p>
  </section>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: var(--xq-sp-4);
}

.card {
  padding: var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-card);
}

.card h2 {
  margin: 0 0 var(--xq-sp-2);
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

.reports {
  margin: 0;
  padding: 0;
  list-style: none;
}

.reports li {
  padding: var(--xq-sp-1) 0;
  border-bottom: 1px solid var(--xq-border);
}

.reports li:last-child {
  border-bottom: none;
}

.time {
  display: inline-block;
  min-width: 48px;
  color: var(--xq-text-2);
  font-variant-numeric: tabular-nums;
}

.pending {
  margin-top: var(--xq-sp-5);
  font-size: var(--xq-fs-sm);
}

.error {
  color: var(--xq-danger);
}
</style>
