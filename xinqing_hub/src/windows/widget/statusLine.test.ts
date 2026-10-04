import { describe, expect, it } from 'vitest'
import type { StatusSnapshot } from '@/api'
import { statusLine } from './statusLine'

const ready: StatusSnapshot = {
  state: 'hesitant',
  weather: 'cloudy',
  prob: 0.8,
  offline: false,
  paused: false,
  connected: true,
  baseline_progress: 100,
}

describe('statusLine', () => {
  it('普通状态：天气图标 + 名称 + 不确定说法，概率只放在悬停提示里', () => {
    const l = statusLine(ready)
    expect(l.text).toBe('⛅ 多云 · 看起来有点犹豫')
    expect(l.text).not.toContain('80')
    expect(l.hint).toBe('看起来有点犹豫（可能性 80%）')
    expect(l.eyesClosed).toBe(false)
  })

  it('没有概率时不显示百分比', () => {
    expect(statusLine({ ...ready, prob: null }).hint).toBeNull()
  })

  it('特殊情形按优先级：暂停 > 未连接 > 冷启动', () => {
    const paused = statusLine({ ...ready, paused: true, connected: false, baseline_progress: 10 })
    expect(paused.text).toBe('休息中，不看你打字啦')
    expect(paused.eyesClosed).toBe(true)
    expect(statusLine({ ...ready, connected: false, baseline_progress: 10 }).text).toBe('等待输入法连接…')
    expect(statusLine({ ...ready, baseline_progress: 38 }).text).toBe('正在熟悉你的打字习惯（38%）')
  })

  it('特殊情形下不给概率提示，天气保持不变', () => {
    const l = statusLine({ ...ready, paused: true })
    expect(l.hint).toBeNull()
    expect(l.weather).toBe('cloudy')
  })
})
