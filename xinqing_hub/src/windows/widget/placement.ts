// 小组件窗口的位置计算（07 FR-WGT-01）：默认位置、贴边吸附、按显示器记住位置、贴边隐藏的小标签。
// 纯函数，坐标一律是物理像素（与 Tauri 的 outerPosition / Monitor.workArea 一致），规范里的逻辑像素乘 scale。

export interface Point {
  x: number
  y: number
}

export interface Size {
  w: number
  h: number
}

export type Rect = Point & Size

/** 吸附在左边缘还是右边缘；只有左右边缘才能贴边隐藏。 */
export type Edge = 'left' | 'right'

/** 默认位置距工作区边缘（逻辑像素） */
export const DEFAULT_MARGIN = 16
/** 拖到距工作区边缘这么近时吸附（逻辑像素） */
export const SNAP_DISTANCE = 24
/** 贴边隐藏后小标签的尺寸（逻辑像素）：24 宽，高度只够露出小精灵 */
export const TAB_SIZE: Size = { w: 24, h: 72 }

/** 显示器的最小描述，便于测试时不依赖 Tauri 的类 */
export interface Screen {
  id: string
  primary: boolean
  work: Rect
  scale: number
}

/** 记住的位置：最后所在的显示器，以及在每台显示器上的位置 */
export interface SavedPlacement {
  last: string
  byScreen: Record<string, Point>
}

/** 主显示器工作区右下角，距边缘 16 px。 */
export function defaultPosition(work: Rect, size: Size, scale: number): Point {
  const m = Math.round(DEFAULT_MARGIN * scale)
  return { x: work.x + work.w - size.w - m, y: work.y + work.h - size.h - m }
}

/** 把窗口整个挪回工作区内（分辨率或任务栏变了以后，记住的位置可能已经出界）。 */
export function clampInto(p: Point, size: Size, work: Rect): Point {
  const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi))
  return {
    x: clamp(p.x, work.x, work.x + work.w - size.w),
    y: clamp(p.y, work.y, work.y + work.h - size.h),
  }
}

/**
 * 距工作区某条边不超过 24 px（含已经越过边缘）就贴上去。返回新位置和吸附的左右边缘。
 * 两个方向各自判断，所以拖到角落会同时贴两条边。
 */
export function snap(win: Rect, work: Rect, scale: number): Point & { edge: Edge | null } {
  const d = SNAP_DISTANCE * scale
  const right = work.x + work.w
  const bottom = work.y + work.h
  let { x, y } = win
  let edge: Edge | null = null
  if (win.x - work.x <= d) {
    x = work.x
    edge = 'left'
  } else if (right - (win.x + win.w) <= d) {
    x = right - win.w
    edge = 'right'
  }
  if (win.y - work.y <= d) y = work.y
  else if (bottom - (win.y + win.h) <= d) y = bottom - win.h
  return { x, y, edge }
}

/** 贴边隐藏后的小标签：紧贴吸附的那条边，垂直方向与小组件居中对齐。 */
export function tabRect(win: Rect, work: Rect, edge: Edge, scale: number): Rect {
  const w = Math.round(TAB_SIZE.w * scale)
  const h = Math.round(TAB_SIZE.h * scale)
  const y = clampInto({ x: 0, y: win.y + Math.round((win.h - h) / 2) }, { w, h }, work).y
  return { x: edge === 'left' ? work.x : work.x + work.w - w, y, w, h }
}

/**
 * 启动时放在哪里：最后所在的显示器还在，就回到在那台显示器上记住的位置（挪回工作区内）；
 * 否则（显示器拔掉了、从没记过）回到主显示器的默认位置。
 */
export function restore(saved: SavedPlacement | null, screens: Screen[], size: Size): Point | null {
  const last = saved && screens.find((s) => s.id === saved.last)
  const pos = last && saved.byScreen[last.id]
  if (last && pos) return clampInto(pos, size, last.work)
  const primary = screens.find((s) => s.primary) ?? screens[0]
  return primary ? defaultPosition(primary.work, size, primary.scale) : null
}

/** 记下窗口在某台显示器上的位置，返回新的记录（不改原对象）。 */
export function remember(saved: SavedPlacement | null, screen: string, pos: Point): SavedPlacement {
  return { last: screen, byScreen: { ...saved?.byScreen, [screen]: { x: pos.x, y: pos.y } } }
}

/** 从存储里读出来的东西不可信：形状不对就当没记过。 */
export function parseSaved(raw: string | null): SavedPlacement | null {
  if (!raw) return null
  try {
    const v: unknown = JSON.parse(raw)
    if (typeof v !== 'object' || v === null) return null
    const { last, byScreen } = v as Record<string, unknown>
    if (typeof last !== 'string' || typeof byScreen !== 'object' || byScreen === null) return null
    const out: Record<string, Point> = {}
    for (const [k, p] of Object.entries(byScreen)) {
      const { x, y } = (p ?? {}) as Record<string, unknown>
      if (Number.isFinite(x) && Number.isFinite(y)) out[k] = { x: x as number, y: y as number }
    }
    return { last, byScreen: out }
  } catch {
    return null
  }
}
