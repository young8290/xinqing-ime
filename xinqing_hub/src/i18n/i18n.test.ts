import { describe, expect, it } from 'vitest'
import { errorText, isCopyKey, t } from '.'

describe('t', () => {
  it('替换变量，没给值的变量原样保留', () => {
    expect(t('widget.cold_start', { pct: 38 })).toBe('正在熟悉你的打字习惯（38%）')
    expect(t('widget.cold_start')).toBe('正在熟悉你的打字习惯（{pct}%）')
    expect(t('widget.prob_hint', { hedge: '看起来有点犹豫' })).toBe('看起来有点犹豫（可能性 {pct}%）')
  })

  it('收录 explain.toml，键名加 explain. 前缀', () => {
    expect(t('explain.hedge.hesitant')).toBe('看起来有点犹豫')
    expect(isCopyKey('explain.hedge.tired')).toBe(true)
    expect(isCopyKey('version')).toBe(false)
  })
})

describe('errorText', () => {
  it('按 message_key 取文案', () => {
    expect(errorText({ code: 'x', message_key: 'error.offline' })).toBe(t('error.offline'))
  })

  it('未知键或非命令错误回落通用提示，不显示错误码', () => {
    expect(errorText({ code: 'store', message_key: 'no.such.key' })).toBe(t('error.generic'))
    expect(errorText(new Error('boom'))).toBe(t('error.generic'))
    expect(errorText(undefined)).toBe(t('error.generic'))
  })
})
