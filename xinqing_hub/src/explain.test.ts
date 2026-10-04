import { describe, expect, it } from 'vitest'
import type { Explanation, SignalKind } from '@/api'
import { explainLines } from './explain'

const base: Explanation = {
  state: 'hesitant',
  prob_pct: 85,
  signals: [
    { kind: 'abandon', value: null },
    { kind: 'pause', value: 2 },
    { kind: 'kpm_slow', value: 35 },
  ],
  source: 'jev',
  cold_start: false,
}

describe('explainLines', () => {
  it('规格示例：标题带可能性，每条说明一行，来源单列', () => {
    expect(explainLines(base)).toEqual({
      header: '看起来有点犹豫（可能性 85%）',
      signals: ['有一段话打了又删', '句子中间停顿了 2 次', '打字速度比平时慢 35%'],
      footer: 'AI 根据打字节奏判断，可能不准',
    })
  })

  it('没有可能性时标题只有不确定说法；冷启动附注接在来源后面', () => {
    const l = explainLines({ ...base, prob_pct: null, source: 'rule', cold_start: true })
    expect(l.header).toBe('看起来有点犹豫')
    expect(l.footer).toBe('本地规则判断（离线） · 还在熟悉你的习惯，判断可能不准')
  })

  it('没有说明时显示“节奏和平时差不多”，保证至少一条（KPI-10）', () => {
    expect(explainLines({ ...base, state: 'fluent', signals: [] }).signals).toEqual(['节奏和平时差不多'])
  })

  it('每种说明都有文案，带值的都把值填进去', () => {
    const kinds: SignalKind[] = [
      'kpm_slow',
      'kpm_fast',
      'iki_long',
      'iki_messy',
      'pause',
      'abandon',
      'delete_committed',
      'bs_more',
      'typo',
      'page_flips',
      'session',
      'late',
    ]
    for (const kind of kinds) {
      const [text] = explainLines({ ...base, signals: [{ kind, value: 7 }] }).signals
      expect(text, kind).toBeTruthy()
      expect(text, kind).not.toMatch(/[{}]/)
      expect(text, kind).not.toContain(`explain.signal`)
    }
  })

  it('不出现“你现在很…”式的断言（DS-COPY-02）', () => {
    expect(explainLines(base).header).not.toMatch(/^你/)
  })
})
