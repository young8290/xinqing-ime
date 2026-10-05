import { beforeEach, describe, expect, it } from 'vitest'
import { loadCorner, requestGreeting, saveCorner, takeGreeting } from './firstRun'

describe('首次引导交给小组件的偏好（FR-ONB-05）', () => {
  beforeEach(() => localStorage.clear())

  it('没选过是右下角；存了乱值也当右下角', () => {
    expect(loadCorner()).toBe('br')
    localStorage.setItem('xq.widget.corner', 'middle')
    expect(loadCorner()).toBe('br')
    saveCorner('tr')
    expect(loadCorner()).toBe('tr')
  })

  it('招呼只打一次', () => {
    expect(takeGreeting()).toBe(false)
    requestGreeting()
    expect(takeGreeting()).toBe(true)
    expect(takeGreeting()).toBe(false)
  })
})
