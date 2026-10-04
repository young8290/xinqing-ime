import { describe, expect, it } from 'vitest'
import type { StatusSnapshot } from '@/api'
import { menuEntries } from './menu'

const snap: StatusSnapshot = {
  state: 'fluent',
  weather: 'sunny',
  prob: null,
  offline: false,
  paused: false,
  connected: true,
  baseline_progress: 100,
}

describe('menuEntries', () => {
  it('按规格顺序：我现在…、暂停感知、打开看板、设置、隐藏小组件', () => {
    expect(menuEntries(snap).map((e) => e.text)).toEqual([
      '我现在…',
      '暂停感知',
      '打开看板',
      '设置',
      '隐藏小组件',
    ])
  })

  it('已暂停时换成“恢复感知”', () => {
    expect(menuEntries({ ...snap, paused: true })[1]).toEqual({ id: 'resume', text: '恢复感知' })
  })

  it('还没拿到状态时按未暂停处理', () => {
    expect(menuEntries(null)[1]?.id).toBe('pause')
  })
})
