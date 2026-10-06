// 看板周报 · 作息洞察（05 FR-REV-03、07 FR-DSH-04）的纯函数：时间格式、纵轴范围、ECharts option。
// 纵轴是“距当晚 0 点的分钟数”（get_routine 的 stop_min：18:00 = 1080，次日 01:30 = 1530），越往上越晚。
// 只说“停止打字时间”，不说睡眠（FR-REV-03 文案约束）。
import type { Routine, RoutineNight } from '@/api'
import type { ChartColors } from '@/components/chartTheme'
import type { ChartOption } from '@/components/EChart.vue'
import { t } from '@/i18n'

/** 0 点，晚于它算“晚于零点” */
export const MIDNIGHT = 1440
const EVENING = 1080
const MORNING = 1800

/** 1530 → "01:30"（只取钟点，不管是哪天） */
export function clock(min: number): string {
  const m = ((Math.round(min) % 1440) + 1440) % 1440
  return `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`
}

/** 1530 → "01:30（次日）"；当晚的不加 */
export function stopTime(min: number): string {
  return min >= MIDNIGHT ? `${clock(min)}${t('dashboard.routine.next_day')}` : clock(min)
}

/** "2026-10-05" → "10/5" */
export function shortDate(date: string): string {
  const [, m, d] = date.split('-')
  return `${Number(m)}/${Number(d)}`
}

/**
 * 纵轴范围：整点对齐，至少覆盖 21:00～01:00（零点参考线总在图里），最多 18:00～06:00。
 * 只有一两晚的数据时也不会挤成一条线。
 */
export function yRange(nights: readonly RoutineNight[]): { min: number; max: number } {
  const values = nights.map((n) => n.stop_min).filter((v): v is number => v !== null)
  const lo = Math.min(1260, ...values) - 30
  const hi = Math.max(MIDNIGHT + 60, ...values) + 30
  return {
    min: Math.max(EVENING, Math.floor(lo / 60) * 60),
    max: Math.min(MORNING, Math.ceil(hi / 60) * 60),
  }
}

/** 读屏用的一句话概括 */
export function routineLabel(r: Routine): string {
  return t('dashboard.routine.chart_label', {
    n: r.nights.length,
    avg: r.avg_stop_min === null ? t('dashboard.routine.no_avg') : stopTime(r.avg_stop_min),
    late: r.late_nights,
  })
}

function tooltipLine(n: RoutineNight): string {
  const head = t('dashboard.routine.night_of', { date: shortDate(n.date) })
  const body =
    n.stop_min === null
      ? t('dashboard.routine.no_record')
      : t('dashboard.routine.stopped_at', { time: stopTime(n.stop_min) })
  return `${head}<br/>${body}`
}

/**
 * 折线：单系列（不要图例，标题已说明画的是什么），2 px 线、8 px 点带 2 px 底色外圈；没记录的晚上断开不连；
 * 一条零点参考线（带文字标签）；网格线一档灰、实线；鼠标移上去显示十字准线和当晚数值。
 */
export function routineOption(r: Routine, c: ChartColors): ChartOption {
  const { min, max } = yRange(r.nights)
  return {
    grid: { left: 52, right: 44, top: 16, bottom: 28 },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'line', lineStyle: { color: c.axisText, width: 1 } },
      backgroundColor: c.tooltipBg,
      borderColor: c.grid,
      textStyle: { color: c.text, fontSize: 13 },
      formatter: (params) => {
        const p = Array.isArray(params) ? params[0] : params
        const night = p ? r.nights[p.dataIndex] : undefined
        return night ? tooltipLine(night) : ''
      },
    },
    xAxis: {
      type: 'category',
      data: r.nights.map((n) => shortDate(n.date)),
      boundaryGap: false,
      axisLine: { lineStyle: { color: c.grid } },
      axisTick: { show: false },
      axisLabel: { color: c.axisText, fontSize: 12, hideOverlap: true },
    },
    yAxis: {
      type: 'value',
      min,
      max,
      interval: 60,
      axisLabel: { color: c.axisText, fontSize: 12, formatter: (v: number) => clock(v) },
      splitLine: { lineStyle: { color: c.grid, width: 1, type: 'solid' } },
    },
    series: [
      {
        type: 'line',
        data: r.nights.map((n) => n.stop_min),
        connectNulls: false,
        symbol: 'circle',
        symbolSize: 8,
        showSymbol: true,
        lineStyle: { color: c.series, width: 2, cap: 'round', join: 'round' },
        itemStyle: { color: c.series, borderColor: c.surface, borderWidth: 2 },
        emphasis: { scale: 1.5 },
        markLine: {
          silent: true,
          symbol: 'none',
          lineStyle: { color: c.axisText, width: 1, type: 'solid' },
          label: {
            color: c.axisText,
            fontSize: 12,
            formatter: t('dashboard.routine.midnight'),
            position: 'end',
          },
          data: [{ yAxis: MIDNIGHT }],
        },
      },
    ],
  }
}
