import { describe, expect, it } from 'vitest'
import {
  clampInto,
  defaultPosition,
  parseSaved,
  remember,
  restore,
  snap,
  tabRect,
  type Rect,
  type Screen,
} from './placement'

// 1920×1080、任务栏在底部 40 px 的工作区
const work: Rect = { x: 0, y: 0, w: 1920, h: 1040 }
const size = { w: 320, h: 168 }
const primary: Screen = { id: 'A', primary: true, work, scale: 1 }
const second: Screen = { id: 'B', primary: false, work: { x: 1920, y: 0, w: 2560, h: 1400 }, scale: 1.5 }

describe('defaultPosition', () => {
  it('工作区右下角，距边缘 16 逻辑像素', () => {
    expect(defaultPosition(work, size, 1)).toEqual({ x: 1920 - 320 - 16, y: 1040 - 168 - 16 })
    expect(defaultPosition(work, { w: 480, h: 252 }, 1.5)).toEqual({ x: 1920 - 480 - 24, y: 1040 - 252 - 24 })
  })
})

describe('defaultPosition 的四个角（首次引导可选，FR-ONB-05）', () => {
  it('右上、左下、左上都距边缘 16 px；restore 没记过位置时用选的角', () => {
    expect(defaultPosition(work, size, 1, 'tr')).toEqual({ x: 1920 - 320 - 16, y: 16 })
    expect(defaultPosition(work, size, 1, 'bl')).toEqual({ x: 16, y: 1040 - 168 - 16 })
    expect(defaultPosition(work, size, 1, 'tl')).toEqual({ x: 16, y: 16 })
    expect(restore(null, [primary], size, 'tl')).toEqual({ x: 16, y: 16 })
  })
})

describe('snap', () => {
  it('距边缘 24 px 以内吸附到左边，记下边缘', () => {
    expect(snap({ ...size, x: 20, y: 400 }, work, 1)).toEqual({ x: 0, y: 400, edge: 'left' })
  })

  it('越过边缘也拉回来', () => {
    expect(snap({ ...size, x: -50, y: 400 }, work, 1)).toEqual({ x: 0, y: 400, edge: 'left' })
    expect(snap({ ...size, x: 1700, y: 400 }, work, 1)).toEqual({ x: 1600, y: 400, edge: 'right' })
  })

  it('超过 24 px 不动', () => {
    expect(snap({ ...size, x: 25, y: 400 }, work, 1)).toEqual({ x: 25, y: 400, edge: null })
  })

  it('阈值随缩放放大', () => {
    expect(snap({ ...size, x: 30, y: 400 }, work, 1.5).edge).toBe('left')
  })

  it('上下边缘也吸附，但不算贴边隐藏的边缘', () => {
    expect(snap({ ...size, x: 500, y: 1040 - 168 - 10 }, work, 1)).toEqual({ x: 500, y: 872, edge: null })
    expect(snap({ ...size, x: 500, y: 5 }, work, 1)).toEqual({ x: 500, y: 0, edge: null })
  })

  it('副屏坐标不从 0 开始', () => {
    expect(snap({ ...size, x: 1930, y: 100 }, second.work, 1)).toMatchObject({ x: 1920, edge: 'left' })
  })
})

describe('tabRect', () => {
  it('右边缘的小标签贴边、垂直居中', () => {
    expect(tabRect({ ...size, x: 1600, y: 400 }, work, 'right', 1)).toEqual({ x: 1896, y: 448, w: 24, h: 72 })
  })

  it('左边缘按缩放算尺寸，不出工作区', () => {
    expect(tabRect({ ...size, x: 0, y: -100 }, work, 'left', 2)).toEqual({ x: 0, y: 0, w: 48, h: 144 })
  })
})

describe('restore', () => {
  it('从没记过：主显示器默认位置', () => {
    expect(restore(null, [second, primary], size)).toEqual(defaultPosition(work, size, 1))
  })

  it('最后所在的显示器还在：回到那台显示器上记住的位置', () => {
    const saved = remember(remember(null, 'A', { x: 10, y: 10 }), 'B', { x: 2000, y: 300 })
    expect(restore(saved, [primary, second], size)).toEqual({ x: 2000, y: 300 })
  })

  it('显示器拔掉了：回到主显示器默认位置', () => {
    const saved = remember(null, 'B', { x: 2000, y: 300 })
    expect(restore(saved, [primary], size)).toEqual(defaultPosition(work, size, 1))
  })

  it('记住的位置出界了（分辨率变小）：挪回工作区内', () => {
    const saved = remember(null, 'A', { x: 3000, y: -20 })
    expect(restore(saved, [primary], size)).toEqual({ x: 1600, y: 0 })
  })

  it('一台显示器都拿不到时不动', () => {
    expect(restore(null, [], size)).toBeNull()
  })
})

describe('clampInto', () => {
  it('窗口比工作区还大时贴左上角', () => {
    expect(clampInto({ x: 50, y: 50 }, { w: 400, h: 400 }, { x: 0, y: 0, w: 300, h: 300 })).toEqual({
      x: 0,
      y: 0,
    })
  })
})

describe('parseSaved', () => {
  it('往返', () => {
    const s = remember(null, 'A', { x: 1, y: 2 })
    expect(parseSaved(JSON.stringify(s))).toEqual(s)
  })

  it('坏数据当作没记过，坏条目丢掉', () => {
    expect(parseSaved(null)).toBeNull()
    expect(parseSaved('{')).toBeNull()
    expect(parseSaved('[]')).toBeNull()
    expect(parseSaved('{"last":1,"byScreen":{}}')).toBeNull()
    expect(parseSaved('{"last":"A","byScreen":{"A":{"x":"1","y":2},"B":{"x":3,"y":4}}}')).toEqual({
      last: 'A',
      byScreen: { B: { x: 3, y: 4 } },
    })
  })
})
