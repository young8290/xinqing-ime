// 天气与状态的展示映射（07 DS-COLOR 天气表、DS-COPY-02）。状态本身由后端给出，前端不推断（17 第 3.2 节）。
import type { MoodState, Weather } from '@/api'
import { t, type CopyKey } from '@/i18n'

/** 天气图标（与名称一起显示，不能只靠颜色区分，DS-COLOR-02）。 */
export const WEATHER_ICON: Record<Weather, string> = {
  sunny: '☀️',
  wind: '🌬',
  cloudy: '⛅',
  rain: '🌧',
  storm: '⛈',
  night: '🌙',
}

export function weatherName(w: Weather): string {
  return t(`weather.${w}` satisfies CopyKey)
}

/** 填充色令牌，只用于插画和色块。 */
export function weatherColor(w: Weather): string {
  return `var(--xq-w-${w})`
}

/** 不确定说法，例如“看起来有点犹豫”。`unknown` 不会作为显示状态下发，兜底用 fluent。 */
export function hedge(state: MoodState): string {
  return t(state === 'unknown' ? 'explain.hedge.fluent' : (`explain.hedge.${state}` satisfies CopyKey))
}
