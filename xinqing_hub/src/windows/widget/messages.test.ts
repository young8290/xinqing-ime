import { describe, expect, it } from 'vitest'
import { COMFORT_MS, current, drop, endOfDay, message, nextChange, prune, put, putComfort } from './messages'

const NOW = new Date(2026, 9, 9, 15, 0).getTime()

describe('一句话区的消息优先级（FR-WGT-04）', () => {
  it('高优先级盖住低优先级，低优先级留着等', () => {
    let list = [message('idle', '今天也辛苦啦', NOW)]
    expect(current(list, NOW)?.text).toBe('今天也辛苦啦')
    list = putComfort(list, { id: 7, text: '慢慢来。', ai: true }, NOW)
    expect(current(list, NOW)).toMatchObject({ kind: 'comfort', comfortId: 7, ai: true })
    list = put(list, message('safety', '要不要和晴晴聊聊？', NOW, { action: { type: 'safety' } }))
    expect(current(list, NOW)?.kind).toBe('safety')
    list = drop(list, 'safety')
    expect(current(list, NOW)?.kind).toBe('comfort')
  })

  it('同种消息新的替换旧的', () => {
    let list = putComfort([], { id: 1, text: '旧', ai: false }, NOW)
    list = putComfort(list, { id: 2, text: '新', ai: false }, NOW + 1)
    expect(list.filter((m) => m.kind === 'comfort')).toHaveLength(1)
    expect(current(list, NOW + 1)?.comfortId).toBe(2)
  })

  it('暖心话 2 小时后淡出为当天的今日一句，到午夜为止（FR-CMF-04 第 3 条）', () => {
    const list = [
      message('idle', '问候', NOW),
      ...putComfort([], { id: 3, text: '慢慢来。', ai: false }, NOW),
    ]
    expect(nextChange(list, NOW)).toBe(NOW + COMFORT_MS)
    const later = current(list, NOW + COMFORT_MS)
    expect(later).toMatchObject({ kind: 'today', text: '慢慢来。', comfortId: 3 })
    expect(current(list, endOfDay(NOW))?.kind).toBe('idle')
    expect(prune(list, endOfDay(NOW)).map((m) => m.kind)).toEqual(['idle'])
  })

  it('出错提示压过暖心话，但不压过求助入口', () => {
    let list = putComfort([], { id: 1, text: '慢慢来。', ai: false }, NOW)
    list = put(list, message('error', '出了点小问题', NOW, { until: NOW + 8000 }))
    expect(current(list, NOW)?.kind).toBe('error')
    expect(current(list, NOW + 8000)?.kind).toBe('comfort')
    list = put(list, message('safety', '求助', NOW))
    expect(current(list, NOW)?.kind).toBe('safety')
  })

  it('endOfDay 是本地的下一个零点', () => {
    const end = new Date(endOfDay(NOW))
    expect([end.getHours(), end.getMinutes(), end.getDate()]).toEqual([0, 0, 10])
  })
})
