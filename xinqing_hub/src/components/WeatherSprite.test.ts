import { describe, expect, it } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import type { Weather } from '@/api'
import WeatherSprite from './WeatherSprite.vue'

const ALL: Weather[] = ['sunny', 'wind', 'cloudy', 'rain', 'storm', 'night']

describe('WeatherSprite', () => {
  it('每种天气都有读屏名称和区别于颜色的配饰形状（DS-COLOR-02、DS-A11Y-02）', () => {
    const shapes = new Set<string>()
    for (const weather of ALL) {
      const w = mount(WeatherSprite, { props: { weather } })
      expect(w.attributes('role')).toBe('img')
      expect(w.attributes('aria-label')).toMatch(/^心晴：/)
      shapes.add(w.find('.accessory').html())
    }
    expect(shapes.size).toBe(ALL.length)
  })

  it('读屏名称形如“心晴：多云”', () => {
    expect(mount(WeatherSprite, { props: { weather: 'cloudy' } }).attributes('aria-label')).toBe('心晴：多云')
  })

  it('暂停感知时闭眼', () => {
    const open = mount(WeatherSprite, { props: { weather: 'sunny' } })
    const closed = mount(WeatherSprite, { props: { weather: 'sunny', eyesClosed: true } })
    expect(open.findAll('.face circle')).toHaveLength(2)
    expect(closed.findAll('.face circle')).toHaveLength(0)
  })

  it('play() 播放一次性动作，动画结束后清掉，同一动作可以再播', async () => {
    const w = mount(WeatherSprite, { props: { weather: 'sunny' } })
    const frame = () => new Promise((r) => requestAnimationFrame(r))
    ;(w.vm as unknown as { play: (a: string) => void }).play('shake')
    await frame()
    await flushPromises()
    expect(w.classes()).toContain('is-shake')
    // 循环动画的 animationend 不影响一次性动作（减少动画时循环也会触发一次）
    w.element.dispatchEvent(Object.assign(new Event('animationend'), { animationName: 'xq-breathe-abc' }))
    await flushPromises()
    expect(w.classes()).toContain('is-shake')
    w.element.dispatchEvent(Object.assign(new Event('animationend'), { animationName: 'xq-once-shake-abc' }))
    await flushPromises()
    expect(w.classes()).not.toContain('is-shake')
    ;(w.vm as unknown as { play: (a: string) => void }).play('approach')
    await frame()
    await flushPromises()
    expect(w.classes()).toContain('is-approach')
  })

  it('睁眼时才有眨眼的那组眼睛', () => {
    expect(
      mount(WeatherSprite, { props: { weather: 'rain' } })
        .find('.eyes.open')
        .exists(),
    ).toBe(true)
    expect(
      mount(WeatherSprite, { props: { weather: 'rain', eyesClosed: true } })
        .find('.eyes.open')
        .exists(),
    ).toBe(false)
  })
})
