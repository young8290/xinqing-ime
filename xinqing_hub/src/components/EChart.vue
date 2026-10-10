<script setup lang="ts">
// ECharts 的薄封装（03 第 5 节技术栈：ECharts 5.x）。按需注册用到的图表与组件，用 SVG 渲染（文字清晰、跟随缩放）。
// 颜色一律从设计令牌读（chartColors），深浅色、高对比度切换时重画；用户开了“减少动态效果”就不放动画（DS-MOTION）。
// 用法：传一个“颜色 → option”的函数，组件负责初始化、尺寸变化、主题变化和卸载。
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { BarChart, HeatmapChart, LineChart, ScatterChart } from 'echarts/charts'
import {
  GridComponent,
  MarkLineComponent,
  TooltipComponent,
  VisualMapContinuousComponent,
} from 'echarts/components'
import { init, use, type ComposeOption, type ECharts } from 'echarts/core'
import { SVGRenderer } from 'echarts/renderers'
import type {
  BarSeriesOption,
  HeatmapSeriesOption,
  LineSeriesOption,
  ScatterSeriesOption,
} from 'echarts/charts'
import type {
  GridComponentOption,
  MarkLineComponentOption,
  TooltipComponentOption,
  VisualMapComponentOption,
} from 'echarts/components'
import { chartColors, type ChartColors } from './chartTheme'

// 看板用到的：折线（作息洞察、打字心电图）、条形（周报的输入时长与状态分布）、热力图（低落与疲劳的时段）、散点（心电图上的退格标记）
use([
  LineChart,
  BarChart,
  HeatmapChart,
  ScatterChart,
  GridComponent,
  TooltipComponent,
  MarkLineComponent,
  VisualMapContinuousComponent,
  SVGRenderer,
])

export type ChartOption = ComposeOption<
  | LineSeriesOption
  | BarSeriesOption
  | HeatmapSeriesOption
  | ScatterSeriesOption
  | GridComponentOption
  | TooltipComponentOption
  | MarkLineComponentOption
  | VisualMapComponentOption
>

const props = defineProps<{
  option: (c: ChartColors) => ChartOption
  /** 给读屏的一句话概括；具体数值放在图下的表格里 */
  label: string
}>()

const el = ref<HTMLDivElement | null>(null)
let chart: ECharts | null = null
let resize: ResizeObserver | null = null
let themeWatch: MutationObserver | null = null
const darkQuery = window.matchMedia?.('(prefers-color-scheme: dark)')
const contrastQuery = window.matchMedia?.('(forced-colors: active)')

function reducedMotion(): boolean {
  return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
}

function render(): void {
  if (!chart) return
  chart.setOption({ animation: !reducedMotion(), ...props.option(chartColors()) }, { notMerge: true })
}

onMounted(() => {
  if (!el.value) return
  chart = init(el.value, undefined, { renderer: 'svg' })
  render()
  if (typeof ResizeObserver !== 'undefined') {
    resize = new ResizeObserver(() => chart?.resize())
    resize.observe(el.value)
  }
  // 主题由 <html data-theme> 或系统深浅色、高对比度决定（mount.ts、tokens.css）
  themeWatch = new MutationObserver(render)
  themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
  darkQuery?.addEventListener('change', render)
  contrastQuery?.addEventListener('change', render)
})

watch(() => props.option, render)

onBeforeUnmount(() => {
  resize?.disconnect()
  themeWatch?.disconnect()
  darkQuery?.removeEventListener('change', render)
  contrastQuery?.removeEventListener('change', render)
  chart?.dispose()
  chart = null
})
</script>

<template>
  <div ref="el" class="chart" role="img" :aria-label="props.label" />
</template>

<style scoped>
.chart {
  width: 100%;
  height: 100%;
}
</style>
