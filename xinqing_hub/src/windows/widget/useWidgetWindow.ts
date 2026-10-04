// 小组件窗口行为（07 FR-WGT-01）：启动时回到记住的位置；拖动停下后吸附并记住；
// 吸附在左右边缘且开了贴边隐藏时，鼠标离开 3 秒收成小标签，移入展开；置顶跟随设置。
// 位置计算都在 placement.ts（纯函数，有单测），这里只管调 Tauri 窗口接口。
import { nextTick, onBeforeUnmount, ref, watch, type Ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  PhysicalPosition,
  PhysicalSize,
  availableMonitors,
  currentMonitor,
  getCurrentWindow,
  primaryMonitor,
  type Monitor,
} from '@tauri-apps/api/window'
import {
  parseSaved,
  remember,
  restore,
  snap,
  tabRect,
  type Edge,
  type Rect,
  type SavedPlacement,
  type Screen,
} from './placement'

/** 窗口位置只是本机的界面偏好，不是用户数据，放 WebView 的 localStorage（读不到就当没记过）。 */
const STORAGE_KEY = 'xq.widget.placement'
/** 最后一次移动事件之后这么久没再动，才算拖动停下 */
const SETTLE_MS = 400
/** 贴边隐藏：鼠标离开后这么久收起（FR-WGT-01） */
const AUTOHIDE_MS = 3000

function load(): SavedPlacement | null {
  try {
    return parseSaved(localStorage.getItem(STORAGE_KEY))
  } catch {
    return null
  }
}

function save(p: SavedPlacement): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(p))
  } catch {
    // 存不下就下次回到默认位置，不影响使用
  }
}

/** 显示器名（Windows 上形如 `\\.\DISPLAY1`）拿不到时退回用左上角坐标区分。 */
function screenId(m: Monitor): string {
  return m.name ?? `${m.position.x},${m.position.y}`
}

function toScreen(m: Monitor, primary: Monitor | null = null): Screen {
  const { position, size } = m.workArea
  return {
    id: screenId(m),
    primary: primary !== null && screenId(primary) === screenId(m),
    work: { x: position.x, y: position.y, w: size.width, h: size.height },
    scale: m.scaleFactor,
  }
}

export interface WidgetWindowOptions {
  /** 设置 `widget.autohide` */
  autohide: Ref<boolean>
  /** 设置 `widget.topmost` */
  topmost: Ref<boolean>
  /** 为真时不收起（例如右键菜单开着） */
  holdOpen: Ref<boolean>
}

export function useWidgetWindow(opts: WidgetWindowOptions) {
  // 用到时再取：普通浏览器里没有 Tauri，getCurrentWindow() 会同步抛错，放在 async 函数里才能被 catch 住
  const win = () => getCurrentWindow()
  /** 是否已收成贴边小标签；界面据此只画小精灵 */
  const collapsed = ref(false)
  let edge: Edge | null = null
  let expanded: Rect | null = null
  // 收起 / 展开会连发几次移动和缩放，期间不做吸附，也不把小标签的位置当成小组件的位置记下来
  let busy = false
  let hovering = false
  let settleTimer: ReturnType<typeof setTimeout> | undefined
  let hideTimer: ReturnType<typeof setTimeout> | undefined
  let unlisten: UnlistenFn | null = null

  async function windowRect(): Promise<Rect> {
    const [p, s] = await Promise.all([win().outerPosition(), win().outerSize()])
    return { x: p.x, y: p.y, w: s.width, h: s.height }
  }

  async function settle(): Promise<void> {
    if (busy || collapsed.value) return
    const m = await currentMonitor()
    if (!m) return
    const screen = toScreen(m)
    const r = await windowRect()
    const s = snap(r, screen.work, screen.scale)
    if (s.x !== r.x || s.y !== r.y) await win().setPosition(new PhysicalPosition(s.x, s.y))
    edge = s.edge
    save(remember(load(), screen.id, s))
    scheduleHide()
  }

  function scheduleHide(): void {
    clearTimeout(hideTimer)
    if (!opts.autohide.value || !edge || collapsed.value || hovering || opts.holdOpen.value) return
    hideTimer = setTimeout(() => void collapse().catch(warn), AUTOHIDE_MS)
  }

  async function collapse(): Promise<void> {
    if (busy || collapsed.value || !edge || hovering || opts.holdOpen.value) return
    const m = await currentMonitor()
    if (!m) return
    busy = true
    try {
      const screen = toScreen(m)
      const r = await windowRect()
      const tab = tabRect(r, screen.work, edge, screen.scale)
      expanded = r
      collapsed.value = true
      await nextTick()
      // 先缩小再挪到边上，免得过程中窗口跨到相邻显示器
      await win().setSize(new PhysicalSize(tab.w, tab.h))
      await win().setPosition(new PhysicalPosition(tab.x, tab.y))
    } finally {
      busy = false
    }
  }

  async function expand(): Promise<void> {
    if (busy || !collapsed.value || !expanded) return
    busy = true
    try {
      // 与收起相反：先挪回去再放大
      await win().setPosition(new PhysicalPosition(expanded.x, expanded.y))
      await win().setSize(new PhysicalSize(expanded.w, expanded.h))
      collapsed.value = false
    } finally {
      busy = false
    }
  }

  async function start(): Promise<void> {
    const w = win() // 先取窗口：它同步抛错时不要留下已经发出、没人接的显示器查询
    const [monitors, primary, size] = await Promise.all([
      availableMonitors(),
      primaryMonitor(),
      w.outerSize(),
    ])
    const screens = monitors.map((m) => toScreen(m, primary))
    const pos = restore(load(), screens, { w: size.width, h: size.height })
    if (pos) await w.setPosition(new PhysicalPosition(pos.x, pos.y))
    await settle()
    unlisten = await w.onMoved(() => {
      clearTimeout(settleTimer)
      settleTimer = setTimeout(() => void settle().catch(warn), SETTLE_MS)
    })
  }

  function pointerEnter(): void {
    hovering = true
    clearTimeout(hideTimer)
    void expand().catch(warn)
  }

  function pointerLeave(): void {
    hovering = false
    scheduleHide()
  }

  watch(opts.autohide, (on) => {
    if (on) scheduleHide()
    else {
      clearTimeout(hideTimer)
      void expand().catch(warn)
    }
  })
  watch(opts.holdOpen, (hold) => (hold ? clearTimeout(hideTimer) : scheduleHide()))
  watch(opts.topmost, (on) => void (async () => win().setAlwaysOnTop(on))().catch(warn), { immediate: true })

  onBeforeUnmount(() => {
    unlisten?.()
    clearTimeout(settleTimer)
    clearTimeout(hideTimer)
  })

  return {
    collapsed,
    start: () => start().catch(warn),
    expand: () => expand().catch(warn),
    pointerEnter,
    pointerLeave,
  }
}

function warn(e: unknown): void {
  // 普通浏览器里预览（pnpm dev）没有窗口接口，这里会失败，不影响界面
  console.warn('小组件窗口操作失败', e)
}
