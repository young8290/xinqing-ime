// 看板 · 周报（07 FR-DSH-04）的纯函数与图表 option。
// 状态分布用单色横条（每条带天气图标、名称和百分比）而不是环形图：天气色是插画用的填充色，
// 按图表配色检查（dataviz 校验脚本）多云和小雨在正常视力下都分不开（ΔE 7.0），不能拿来区分系列；
// 单色条形加文字标签，读数也比环形图准。热力图是单色深浅（越深越多），周一在最上面。
import type { StateCount, WeekDay, WeekReport } from '@/api'
import type { ChartColors } from '@/components/chartTheme'
import type { ChartOption } from '@/components/EChart.vue'
import { duration, parseDate, shortDay } from '@/format'
import { t } from '@/i18n'
import { STATE_WEATHER, WEATHER_ICON, weatherName } from '@/weather'

const WEEKDAY = new Intl.DateTimeFormat('zh-CN', { weekday: 'short' })

/** “周一” */
export function weekday(date: string): string {
  const d = parseDate(date)
  return d ? WEEKDAY.format(d) : date
}

/** “10月5日 – 10月11日” */
export function weekRange(r: WeekReport): string {
  const first = r.days[0]?.date ?? r.week_start
  const last = r.days.at(-1)?.date ?? r.week_start
  return t('dashboard.weekly.range', { from: shortDay(first), to: shortDay(last) })
}

export interface DistRow {
  state: StateCount['state']
  /** “☀️ 晴” */
  name: string
  pct: number
  windows: number
}

/** 状态分布：每种状态一行，按固定顺序（后端给的顺序），百分比四舍五入 */
export function distRows(states: readonly StateCount[]): DistRow[] {
  const total = states.reduce((n, s) => n + s.windows, 0)
  return states.flatMap((s) => {
    const w = STATE_WEATHER[s.state]
    if (!w || total === 0) return []
    return [
      {
        state: s.state,
        name: `${WEATHER_ICON[w]} ${weatherName(w)}`,
        pct: Math.round((s.windows * 100) / total),
        windows: s.windows,
      },
    ]
  })
}

export function distLabel(rows: readonly DistRow[]): string {
  const parts = rows.map((r) => t('dashboard.weekly.dist_part', { weather: r.name, pct: r.pct })).join('，')
  return t('dashboard.weekly.dist_label', { parts })
}

const axisText = (c: ChartColors) => ({ color: c.axisText, fontSize: 12 })

function tooltip(c: ChartColors) {
  return {
    backgroundColor: c.tooltipBg,
    borderColor: c.grid,
    textStyle: { color: c.text, fontSize: 13 },
  }
}

/** 状态分布：横条，一条一种状态，单色；条尾直接标百分比（只有 ≤ 5 条，不会挤） */
export function distOption(rows: readonly DistRow[], c: ChartColors): ChartOption {
  return {
    grid: { left: 72, right: 48, top: 8, bottom: 8 },
    tooltip: {
      trigger: 'item',
      ...tooltip(c),
      formatter: (p) =>
        `${rows[(p as { dataIndex: number }).dataIndex]?.name ?? ''}：${(p as { value: number }).value}%`,
    },
    xAxis: { type: 'value', max: 100, show: false },
    yAxis: {
      type: 'category',
      inverse: true,
      data: rows.map((r) => r.name),
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: { ...axisText(c), color: c.text, fontSize: 13 },
    },
    series: [
      {
        type: 'bar',
        data: rows.map((r) => r.pct),
        barMaxWidth: 16,
        itemStyle: { color: c.series, borderRadius: [0, 4, 4, 0] },
        label: { show: true, position: 'right', color: c.text, fontSize: 12, formatter: '{c}%' },
      },
    ],
  }
}

/** 每日输入时长：竖条，单色，4 px 圆角的顶、平的底；鼠标移上去看具体时长 */
export function typingOption(days: readonly WeekDay[], c: ChartColors): ChartOption {
  return {
    grid: { left: 48, right: 16, top: 16, bottom: 28 },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow', shadowStyle: { color: c.empty } },
      ...tooltip(c),
      formatter: (params) => {
        const p = Array.isArray(params) ? params[0] : params
        const d = p ? days[p.dataIndex] : undefined
        return d ? `${weekday(d.date)} ${shortDay(d.date)}<br/>${duration(d.typing_min)}` : ''
      },
    },
    xAxis: {
      type: 'category',
      data: days.map((d) => weekday(d.date)),
      axisLine: { lineStyle: { color: c.grid } },
      axisTick: { show: false },
      axisLabel: axisText(c),
    },
    yAxis: {
      type: 'value',
      minInterval: 60,
      axisLabel: { ...axisText(c), formatter: (v: number) => (v === 0 ? '0' : `${Math.round(v / 6) / 10}h`) },
      splitLine: { lineStyle: { color: c.grid, width: 1, type: 'solid' } },
    },
    series: [
      {
        type: 'bar',
        data: days.map((d) => d.typing_min),
        barMaxWidth: 24,
        itemStyle: { color: c.series, borderRadius: [4, 4, 0, 0] },
      },
    ],
  }
}

/** 0–1 之间在两个 #rrggbb 之间取色；不是十六进制（高对比度下的系统色）就用后者 */
export function mix(a: string, b: string, k: number): string {
  const hex = (s: string) =>
    /^#[0-9a-f]{6}$/i.test(s) ? [1, 3, 5].map((i) => parseInt(s.slice(i, i + 2), 16)) : null
  const x = hex(a)
  const y = hex(b)
  if (!x || !y) return b
  return `#${x
    .map((v, i) =>
      Math.round(v + (y[i]! - v) * k)
        .toString(16)
        .padStart(2, '0'),
    )
    .join('')}`
}

/**
 * 7 × 24 热力图：周一在最上面，0–23 时从左到右；单一色相，越多越深（从底色往系列色过渡），
 * 0 的格子用比底色深一档的灰；格子之间 2 px 底色的缝。
 */
export function heatOption(heat: readonly number[][], days: readonly WeekDay[], c: ChartColors): ChartOption {
  const max = Math.max(1, ...heat.flat())
  const data = heat.flatMap((row, d) => row.map((n, h) => [h, d, n]))
  return {
    grid: { left: 40, right: 8, top: 8, bottom: 28 },
    tooltip: {
      ...tooltip(c),
      formatter: (p) => {
        const [h, d, n] = (p as { value: number[] }).value as [number, number, number]
        const day = days[d]
        return `${day ? weekday(day.date) : ''} ${t('dashboard.weekly.heat_hour', { h })}：${n}`
      },
    },
    xAxis: {
      type: 'category',
      data: Array.from({ length: 24 }, (_, h) => String(h)),
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: { ...axisText(c), interval: 2 },
      splitArea: { show: false },
    },
    yAxis: {
      type: 'category',
      inverse: true,
      data: days.map((d) => weekday(d.date)),
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: axisText(c),
    },
    visualMap: {
      show: false,
      min: 0,
      max,
      dimension: 2,
      inRange: { color: [c.empty, mix(c.surface, c.series, 0.35), c.series] },
    },
    series: [
      {
        type: 'heatmap',
        data,
        itemStyle: { borderColor: c.surface, borderWidth: 2, borderRadius: 2 },
        emphasis: { itemStyle: { borderColor: c.text, borderWidth: 1 } },
      },
    ],
  }
}

/** 和上周比：“比上周多 3 次”/“比上周少 5%”/“和上周一样”；上周没数据时说清楚 */
export function versus(cur: number | null, prev: number | null, unit: (n: number) => string): string {
  if (cur === null) return ''
  if (prev === null) return t('dashboard.weekly.no_last')
  const d = cur - prev
  if (d === 0) return t('dashboard.weekly.same')
  return t(d > 0 ? 'dashboard.weekly.more' : 'dashboard.weekly.less', { n: unit(Math.abs(d)) })
}

/** 一周里有没有任何数据（没有时上周比较不写数字） */
export function hasData(r: WeekReport | null): boolean {
  return !!r && r.days.some((d) => d.typing_min > 0 || d.rests_due > 0)
}
