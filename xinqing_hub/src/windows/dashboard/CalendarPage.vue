<script setup lang="ts">
// 看板 · 情绪日历（07 FR-DSH-03）：月视图，每天显示主导天气图标（打字太少的天显示“—”），有日记的天有 📓；
// 点某天：当天的状态时间线、暖心话、休息提醒完成情况、日记（FR-DIA-03“点开即可查看”）。
import { computed, onMounted, ref, watch } from 'vue'
import { commands, unwrap, type DayMood, type DayStats, type DiaryItem, type SelfReportItem } from '@/api'
import { dayLabel, duration, localDate } from '@/format'
import { errorText, t } from '@/i18n'
import ComfortList from './ComfortList.vue'
import StateTimeline from './StateTimeline.vue'
import { WEEK_HEAD, monthGrid, monthLabel, monthOf, shiftMonth } from './calendar'

const today = localDate(new Date())
const month = ref(monthOf(new Date()))
const moods = ref<DayMood[]>([])
const error = ref<string | null>(null)
const picked = ref<string | null>(null)
const day = ref<{ stats: DayStats; reports: SelfReportItem[]; diaries: DiaryItem[] } | null>(null)

const weeks = computed(() => monthGrid(month.value, moods.value, today))

async function loadMonth(): Promise<void> {
  error.value = null
  try {
    moods.value = await unwrap(commands.monthMoods(month.value))
  } catch (e) {
    error.value = errorText(e)
  }
}

async function open(date: string): Promise<void> {
  picked.value = date
  day.value = null
  try {
    const [stats, reports, diaries] = await Promise.all([
      unwrap(commands.dayStats(date)),
      unwrap(commands.selfReportList(date)),
      unwrap(commands.diaryList()),
    ])
    day.value = { stats, reports, diaries: diaries.filter((d) => d.date === date) }
  } catch (e) {
    error.value = errorText(e)
  }
}

function sourceTag(d: DiaryItem): string | null {
  if (d.source === 'ai_draft') return t('diary.ai_draft')
  if (d.source === 'ai_edited') return t('diary.ai_edited')
  return null
}

onMounted(loadMonth)
watch(month, () => {
  picked.value = null
  void loadMonth()
})
</script>

<template>
  <section>
    <div class="d-head">
      <h1>{{ t('dashboard.nav.calendar') }}</h1>
      <div class="d-row">
        <button :aria-label="t('dashboard.calendar.prev')" @click="month = shiftMonth(month, -1)">
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" class="chev">
            <path d="M10 3 5 8l5 5" />
          </svg>
        </button>
        <span class="month" aria-live="polite">{{ monthLabel(month) }}</span>
        <button :aria-label="t('dashboard.calendar.next')" @click="month = shiftMonth(month, 1)">
          <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" class="chev">
            <path d="m6 3 5 5-5 5" />
          </svg>
        </button>
        <button v-if="month !== monthOf(new Date())" @click="month = monthOf(new Date())">
          {{ t('dashboard.calendar.this_month') }}
        </button>
      </div>
    </div>
    <p class="muted d-small">{{ t('dashboard.calendar.hint') }}</p>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>

    <table class="cal" :aria-label="monthLabel(month)">
      <thead>
        <tr>
          <th v-for="h in WEEK_HEAD" :key="h" scope="col">{{ h }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(week, i) in weeks" :key="i">
          <td v-for="(c, j) in week" :key="j">
            <button
              v-if="c"
              class="cell"
              :class="{ today: c.today, picked: picked === c.date }"
              :aria-label="c.label"
              :aria-pressed="picked === c.date"
              @click="open(c.date)"
            >
              <span class="num">{{ c.day }}</span>
              <span class="icon" aria-hidden="true">{{ c.icon ?? t('dashboard.none') }}</span>
              <span v-if="c.hasDiary" class="diary" aria-hidden="true">{{
                t('dashboard.calendar.diary_mark')
              }}</span>
            </button>
          </td>
        </tr>
      </tbody>
    </table>

    <section v-if="picked" class="d-card detail" :aria-label="dayLabel(picked)">
      <div class="d-head">
        <h2>{{ dayLabel(picked) }}</h2>
        <button @click="picked = null">{{ t('dashboard.calendar.close') }}</button>
      </div>
      <template v-if="day">
        <p class="d-small d-row">
          <span>{{ t('dashboard.calendar.typing', { duration: duration(day.stats.typing_min) }) }}</span>
          <span>{{
            day.stats.rests_due > 0
              ? t('dashboard.calendar.rests', { done: day.stats.rests_done, due: day.stats.rests_due })
              : t('dashboard.calendar.rests_none')
          }}</span>
        </p>
        <StateTimeline :date="picked" :segments="day.stats.timeline" :reports="day.reports" />
        <h3>{{ t('dashboard.today.comfort_title') }}</h3>
        <ComfortList :date="picked" />
        <h3>{{ t('dashboard.calendar.diaries') }}</h3>
        <p v-if="day.diaries.length === 0" class="muted">{{ t('dashboard.calendar.no_diary') }}</p>
        <article v-for="d in day.diaries" :key="d.id" class="diary-text">
          <span v-if="sourceTag(d)" class="d-tag">{{ sourceTag(d) }}</span>
          <p>{{ d.content }}</p>
        </article>
      </template>
    </section>
  </section>
</template>

<style scoped>
.month {
  min-width: 96px;
  font-weight: 500;
  text-align: center;
}

.cal {
  width: 100%;
  table-layout: fixed;
  border-collapse: separate;
  border-spacing: var(--xq-sp-1);
}

.cal th {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
  font-weight: 400;
}

.cell {
  display: flex;
  flex-direction: column;
  gap: 2px;
  align-items: center;
  width: 100%;
  min-height: 64px;
  padding: var(--xq-sp-1);
  border-color: var(--xq-border);
  background: var(--xq-surface);
}

.cell.today {
  border-color: var(--xq-text-2);
  border-width: 2px;
}

.cell.picked {
  background: var(--xq-surface-2);
}

.num {
  align-self: flex-start;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.icon {
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.diary {
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.detail {
  margin-top: var(--xq-sp-4);
}

.detail h3 {
  margin: var(--xq-sp-4) 0 var(--xq-sp-2);
  font-size: var(--xq-fs-md);
  font-weight: 500;
}

.diary-text p {
  margin: var(--xq-sp-1) 0 var(--xq-sp-3);
  white-space: pre-wrap;
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
