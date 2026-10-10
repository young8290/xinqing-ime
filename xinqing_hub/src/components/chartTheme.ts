// 图表用色：从设计令牌读当前值（tokens.css），ECharts 拿不到 CSS 变量，只能在画之前解析成具体颜色。
// 规则（dataviz 规范）：线和点用 --xq-chart-1；网格、坐标轴一档灰（--xq-border），轴上文字用文字令牌，不用系列色。

export interface ChartColors {
  /** 单系列的线与点 */
  series: string
  /** 图表底色：点外圈的 2 px 描边用它，把点和线分开 */
  surface: string
  /** 网格线、坐标轴、参考线 */
  grid: string
  /** 轴标签、参考线标签 */
  axisText: string
  /** 提示框正文 */
  text: string
  /** 提示框底色 */
  tooltipBg: string
  /** 热力图里“0”的格子：比底色深一档，看得出格子在那儿 */
  empty: string
  /** 打字心电图上成串退格的标记（07 FR-DSH-02 规定用红色；另有三角形状与图例，不只靠颜色） */
  alert: string
}

/** 读不到令牌时（测试环境）的回落值，与浅色令牌一致 */
const FALLBACK: ChartColors = {
  series: '#c47a1f',
  surface: '#ffffff',
  grid: '#e7e1d8',
  axisText: '#746e64',
  text: '#2b2a28',
  tooltipBg: '#ffffff',
  empty: '#f6f3ee',
  alert: '#c23a2e',
}

export function chartColors(root: Element = document.documentElement): ChartColors {
  const css = getComputedStyle(root)
  const v = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback
  return {
    series: v('--xq-chart-1', FALLBACK.series),
    surface: v('--xq-surface', FALLBACK.surface),
    grid: v('--xq-border', FALLBACK.grid),
    axisText: v('--xq-text-3', FALLBACK.axisText),
    text: v('--xq-text-1', FALLBACK.text),
    tooltipBg: v('--xq-surface', FALLBACK.tooltipBg),
    empty: v('--xq-surface-2', FALLBACK.empty),
    alert: v('--xq-danger', FALLBACK.alert),
  }
}

export { FALLBACK as FALLBACK_CHART_COLORS }
