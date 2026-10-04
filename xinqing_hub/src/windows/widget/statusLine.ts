// 小组件状态行（07 FR-WGT-03）。特殊情形按优先级覆盖普通的“天气 · 不确定说法”。
import type { StatusSnapshot, Weather } from '@/api'
import { t } from '@/i18n'
import { WEATHER_ICON, hedge, weatherName } from '@/weather'

export interface StatusLine {
  /** 状态行文字 */
  text: string
  /** 悬停提示：只有普通状态且有概率时才显示百分比（DS-COPY-02） */
  hint: string | null
  weather: Weather
  /** 小精灵闭眼（暂停感知） */
  eyesClosed: boolean
}

export function statusLine(s: StatusSnapshot): StatusLine {
  const base = { weather: s.weather, hint: null, eyesClosed: false }
  if (s.paused) return { ...base, text: t('widget.paused'), eyesClosed: true }
  if (!s.connected) return { ...base, text: t('widget.waiting_ime') }
  if (s.baseline_progress < 100) {
    return { ...base, text: t('widget.cold_start', { pct: s.baseline_progress }) }
  }
  const h = hedge(s.state)
  return {
    ...base,
    text: `${WEATHER_ICON[s.weather]} ${weatherName(s.weather)} · ${h}`,
    hint: s.prob === null ? null : t('widget.prob_hint', { hedge: h, pct: Math.round(s.prob * 100) }),
  }
}
