import { describe, expect, it } from 'vitest'
import type { SelfReportItem } from '@/api'
import {
  OVERRIDE_MS,
  SELF_WEATHERS,
  localDate,
  overrideFrom,
  overrideFromList,
  selfLabel,
  selfOption,
} from './selfReport'

const item = (over: Partial<SelfReportItem>): SelfReportItem => ({
  id: 1,
  ts: 1_000,
  weather: 'night',
  note: null,
  auto_state: null,
  ...over,
})

describe('自评文案', () => {
  it('六个选项按规格顺序，按钮带 emoji，状态行去掉 emoji', () => {
    expect(SELF_WEATHERS.map(selfOption)).toEqual([
      '☀️ 挺好',
      '⛅ 有点犹豫',
      '🌧 有点低落',
      '⛈ 有点烦',
      '🌙 有点累',
      '🤷 说不上来',
    ])
    expect(selfLabel('night')).toBe('有点累')
    expect(selfLabel('sunny')).toBe('挺好')
  })
})

describe('覆盖期', () => {
  it('“说不上来”不覆盖，过期也不覆盖', () => {
    expect(overrideFrom('unsure', 5_000, 1_000)).toBeNull()
    expect(overrideFrom('rain', 1_000, 1_000)).toBeNull()
    expect(overrideFrom('rain', null, 1_000)).toBeNull()
    expect(overrideFrom('rain', 5_000, 1_000)).toEqual({ weather: 'rain', until: 5_000 })
  })

  it('窗口打开时由当天最后一条自评推出：60 分钟内还在覆盖', () => {
    const items = [item({ ts: 0, weather: 'sunny' }), item({ ts: 10_000, weather: 'night' })]
    expect(overrideFromList(items, 20_000)).toEqual({ weather: 'night', until: 10_000 + OVERRIDE_MS })
    expect(overrideFromList(items, 10_000 + OVERRIDE_MS)).toBeNull()
  })

  it('最后一条是“说不上来”：之前的覆盖也结束了', () => {
    const items = [item({ ts: 0, weather: 'night' }), item({ ts: 10, weather: 'unsure' })]
    expect(overrideFromList(items, 20)).toBeNull()
    expect(overrideFromList([], 20)).toBeNull()
  })

  it('本地日期补零', () => {
    expect(localDate(new Date(2026, 0, 5))).toBe('2026-01-05')
  })
})
