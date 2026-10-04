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
  it('普通状态：天气图标 + 名称 + 不确定说法，不显示概率（悬停才显示，DS-COPY-02）', () => {
    const l = statusLine(ready)
    expect(l.text).toBe('⛅ 多云 · 看起来有点犹豫')
    expect(l.text).not.toContain('80')
    expect(l.eyesClosed).toBe(false)
  })

  it('特殊情形按优先级：暂停 > 未连接 > 冷启动', () => {
    const paused = statusLine({ ...ready, paused: true, connected: false, baseline_progress: 10 })
    expect(paused.text).toBe('休息中，不看你打字啦')
    expect(paused.eyesClosed).toBe(true)
    expect(statusLine({ ...ready, connected: false, baseline_progress: 10 }).text).toBe('等待输入法连接…')
    expect(statusLine({ ...ready, baseline_progress: 38 }).text).toBe('正在熟悉你的打字习惯（38%）')
  })

  it('特殊情形下天气保持不变', () => {
    const l = statusLine({ ...ready, paused: true })
    expect(l.weather).toBe('cloudy')
  })

  it('自评覆盖期：显示“你说的：有点累”，天气用自评的（FR-STA-10）', () => {
    const self = { weather: 'night' as const, until: 1 }
    const l = statusLine(ready, self)
    expect(l.text).toBe('🌙 你说的：有点累')
    expect(l.weather).toBe('night')
    // 压过冷启动，但暂停、未连接时不显示心情
    expect(statusLine({ ...ready, baseline_progress: 10 }, self).text).toBe('🌙 你说的：有点累')
    expect(statusLine({ ...ready, paused: true }, self).text).toBe('休息中，不看你打字啦')
    expect(statusLine({ ...ready, connected: false }, self).text).toBe('等待输入法连接…')
  })
})
