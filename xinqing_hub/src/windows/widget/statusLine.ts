// 小组件状态行（07 FR-WGT-03）。特殊情形按优先级覆盖普通的“天气 · 不确定说法”。
// 可能性百分比只在悬停时显示（DS-COPY-02），由解释面板负责（useExplain.ts）。
import type { StatusSnapshot, Weather } from '@/api'
import { t } from '@/i18n'
import { selfLabel, type SelfOverride } from '@/selfReport'
import { WEATHER_ICON, hedge, weatherName } from '@/weather'

export interface StatusLine {
  /** 状态行文字 */
  text: string
  weather: Weather
  /** 小精灵闭眼（暂停感知） */
  eyesClosed: boolean
}

/**
 * 优先级：暂停 > 未连接 > 自评覆盖期（“你说的：有点累”，FR-STA-10）> 冷启动 > 普通。
 * 自评是用户自己说的，所以压过冷启动的提示；但暂停、未连接时不看你打字，也就不显示心情。
 */
export function statusLine(s: StatusSnapshot, self: SelfOverride | null = null): StatusLine {
  const base = { weather: s.weather, eyesClosed: false }
  if (s.paused) return { ...base, text: t('widget.paused'), eyesClosed: true }
  if (!s.connected) return { ...base, text: t('widget.waiting_ime') }
  if (self) {
    const text = `${WEATHER_ICON[self.weather]} ${t('widget.self_reported', { weather: selfLabel(self.weather) })}`
    return { ...base, weather: self.weather, text }
  }
  if (s.baseline_progress < 100) {
    return { ...base, text: t('widget.cold_start', { pct: s.baseline_progress }) }
  }
  return {
    ...base,
    text: `${WEATHER_ICON[s.weather]} ${weatherName(s.weather)} · ${hedge(s.state)}`,
  }
}
