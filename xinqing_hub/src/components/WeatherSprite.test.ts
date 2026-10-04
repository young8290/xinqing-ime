import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
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
})
