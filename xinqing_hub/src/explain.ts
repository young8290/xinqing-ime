// 状态解释的界面拼句（04 FR-STA-09、ADR 0010 第 5 条、DS-COPY-02/11）。
// 后端只给结构（state_explain），这里按 explain.toml 的文案键排成多行：
// 第一行不确定说法（有可能性时附百分比），中间每条说明一行，最后是来源与冷启动附注。
// 小组件悬停和看板时间线共用。
import type { Explanation, MoodState } from '@/api'
import { t, type CopyKey } from '@/i18n'
import { hedge } from '@/weather'

export interface ExplainLines {
  /** “看起来有点犹豫（可能性 85%）”；没有可能性时只有不确定说法 */
  header: string
  /** 每条说明一句；没有显著信号时是“节奏和平时差不多” */
  signals: string[]
  /** “AI 根据打字节奏判断，可能不准 · 还在熟悉你的习惯，判断可能不准” */
  footer: string
}

/** 可能性（0–100 的整数）转成标题行。 */
export function explainHeader(state: MoodState, prob: number | null): string {
  const h = hedge(state)
  return prob === null ? h : t('widget.prob_hint', { hedge: h, pct: prob })
}

export function explainLines(e: Explanation): ExplainLines {
  // 各说明的变量名不同（{p} / {n} / {m}），值只有一个，全给上即可；没用到的变量 t() 不会替换
  const signals = e.signals.map(({ kind, value }) => {
    const v = value ?? ''
    return t(`explain.signal.${kind}.text` satisfies CopyKey, { p: v, n: v, m: v })
  })
  const notes = [t(`explain.source.${e.source}` satisfies CopyKey)]
  if (e.cold_start) notes.push(t('explain.note.cold_start'))
  return {
    header: explainHeader(e.state, e.prob),
    signals: signals.length > 0 ? signals : [t('explain.note.no_signal')],
    footer: notes.join(t('explain.note.separator')),
  }
}
