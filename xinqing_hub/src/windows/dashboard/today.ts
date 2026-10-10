// 看板 · 今日（07 FR-DSH-02）的纯函数：四张概要卡片、状态时间线的几何、打字心电图的 ECharts option。
// 状态与统计都由后端给（day_stats、typing:pulse），这里只排版。
import type { DayStats, Pulse, Segment, SelfReportItem, Weather } from '@/api'
import type { ChartColors } from '@/components/chartTheme'
import type { ChartOption } from '@/components/EChart.vue'
import { clockOf, duration } from '@/format'
import { t } from '@/i18n'
import { selfLabel } from '@/selfReport'
import { STATE_WEATHER, WEATHER_ICON, weatherName } from '@/weather'

const DAY_MS = 24 * 60 * 60 * 1000

export interface SummaryCard {
  key: 'typing' | 'dominant' | 'comforts' | 'rest'
  label: string
  value: string
  detail: string | null
}

/** 四张概要卡片：今日输入时长、主导天气、暖心话次数、休息完成率（FR-DSH-02） */
export function summaryCards(s: DayStats): SummaryCard[] {
  const w = s.dominant ? STATE_WEATHER[s.dominant] : null
  const rate = s.rests_due > 0 ? Math.round((Math.min(s.rests_done, s.rests_due) * 100) / s.rests_due) : null
  return [
    { key: 'typing', label: t('dashboard.today.typing'), value: duration(s.typing_min), detail: null },
    {
      key: 'dominant',
      label: t('dashboard.today.dominant'),
      value: w ? `${WEATHER_ICON[w]} ${weatherName(w)}` : t('dashboard.none'),
      detail: null,
    },
    {
      key: 'comforts',
      label: t('dashboard.today.comforts'),
      value: t('dashboard.today.comforts_value', { n: s.comforts }),
      detail: null,
    },
    {
      key: 'rest',
      label: t('dashboard.today.rest_rate'),
      value: rate === null ? t('dashboard.none') : t('dashboard.today.rest_rate_value', { rate }),
      detail:
        rate === null
          ? t('dashboard.today.rest_none')
          : t('dashboard.today.rest_detail', { done: s.rests_done, due: s.rests_due }),
    },
  ]
}

/** 当天 0 点（Unix 毫秒） */
export function dayStart(date: string): number {
  const [y, m, d] = date.split('-').map(Number)
  return new Date(y!, (m ?? 1) - 1, d ?? 1).getTime()
}

export interface BandSegment {
  seg: Segment
  weather: Weather | null
  /** 在 0–24 时轴上的位置与宽度（百分比） */
  left: number
  width: number
  /** 开始、结束时刻“09:00” */
  from: string
  to: string
  /** “09:00–09:40 多云”：按钮的读屏名称，也是列表视图里的一行 */
  label: string
}

/** 时间线的色带：每段按开始、结束时刻放到 0–24 时轴上（跨出当天的部分截掉）。 */
export function band(segments: readonly Segment[], start: number): BandSegment[] {
  return segments
    .map((seg) => {
      const from = Math.max(seg.start_ts, start)
      const to = Math.min(seg.end_ts, start + DAY_MS)
      const weather = STATE_WEATHER[seg.state]
      const times = { from: clockOf(seg.start_ts), to: clockOf(seg.end_ts) }
      return {
        seg,
        weather,
        left: ((from - start) / DAY_MS) * 100,
        width: (Math.max(0, to - from) / DAY_MS) * 100,
        ...times,
        label: t('dashboard.today.segment', {
          ...times,
          weather: weather ? weatherName(weather) : t('dashboard.none'),
        }),
      }
    })
    .filter((b) => b.width > 0)
}

/** 自评的实心圆点（FR-DSH-02：用户自评以实心圆点标出） */
export function dots(
  reports: readonly SelfReportItem[],
  start: number,
): { left: number; label: string; id: number }[] {
  return reports
    .filter((r) => r.ts >= start && r.ts < start + DAY_MS)
    .map((r) => ({
      id: r.id,
      left: ((r.ts - start) / DAY_MS) * 100,
      label: t('dashboard.today.dot', { time: clockOf(r.ts), weather: selfLabel(r.weather) }),
    }))
}

/** 打字心电图看最近多久（FR-DSH-02：最近 60 秒） */
export const ECG_WINDOW_MS = 60_000
/** 成串退格：连着几次退格才标出来 */
export const BACKSPACE_RUN = 3

/** 只留最近 60 秒的脉冲 */
export function trim<T extends { ts: number }>(items: readonly T[], now: number): T[] {
  return items.filter((p) => p.ts > now - ECG_WINDOW_MS && p.ts <= now)
}

/** 成串退格：连续 ≥ 3 次退格里的每一下 */
export function backspaceRuns(pulses: readonly Pulse[]): Pulse[] {
  const out: Pulse[] = []
  let run: Pulse[] = []
  for (const p of [...pulses, null]) {
    if (p?.backspace) {
      run.push(p)
      continue
    }
    if (run.length >= BACKSPACE_RUN) out.push(...run)
    run = []
  }
  return out
}

/** 读屏用的一句话：最近 60 秒按了几次键、最长停了几秒、退格几次 */
export function ecgLabel(pulses: readonly Pulse[]): string {
  const max = pulses.reduce((m, p) => Math.max(m, p.iki_ms), 0)
  return t('dashboard.today.ecg_label', {
    n: pulses.length,
    max: (max / 1000).toFixed(1),
    bs: pulses.filter((p) => p.backspace).length,
  })
}

/**
 * 打字心电图：横轴最近 60 秒，纵轴键间间隔（秒），长停顿就是尖峰；成串退格用红色三角标在波形上；
 * 状态变化处一条竖线，标签是新天气的图标（FR-DSH-02）。单系列折线不放图例（标题说明），退格标记另在图下写明。
 */
export function ecgOption(
  pulses: readonly Pulse[],
  changes: readonly { ts: number; weather: Weather }[],
  now: number,
  c: ChartColors,
): ChartOption {
  const runs = backspaceRuns(pulses)
  return {
    // 10 Hz 刷新：动画会一直追不上，关掉
    animation: false,
    grid: { left: 40, right: 16, top: 24, bottom: 24 },
    xAxis: {
      type: 'value',
      min: now - ECG_WINDOW_MS,
      max: now,
      interval: 10_000,
      axisLine: { lineStyle: { color: c.grid } },
      axisTick: { show: false },
      axisLabel: {
        color: c.axisText,
        fontSize: 12,
        formatter: (v: number) => (v >= now ? '0' : `-${Math.round((now - v) / 1000)}`),
      },
      splitLine: { show: false },
    },
    yAxis: {
      type: 'value',
      min: 0,
      max: 5,
      interval: 1,
      axisLabel: { color: c.axisText, fontSize: 12, formatter: (v: number) => `${v}s` },
      splitLine: { lineStyle: { color: c.grid, width: 1, type: 'solid' } },
    },
    series: [
      {
        type: 'line',
        data: pulses.map((p) => [p.ts, Math.min(p.iki_ms, 5000) / 1000]),
        showSymbol: false,
        lineStyle: { color: c.series, width: 2, cap: 'round', join: 'round' },
        itemStyle: { color: c.series },
        markLine: {
          silent: true,
          symbol: 'none',
          lineStyle: { color: c.axisText, width: 1, type: 'solid' },
          label: { color: c.text, fontSize: 14, position: 'end', formatter: (p) => String(p.name ?? '') },
          data: changes.map((ch) => ({ xAxis: ch.ts, name: WEATHER_ICON[ch.weather] })),
        },
      },
      {
        type: 'scatter',
        data: runs.map((p) => [p.ts, Math.min(p.iki_ms, 5000) / 1000]),
        symbol: 'triangle',
        symbolSize: 10,
        itemStyle: { color: c.alert, borderColor: c.surface, borderWidth: 2 },
        silent: true,
      },
    ],
  }
}
