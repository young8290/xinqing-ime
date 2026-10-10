// 卡片层窗口的位置（07 第 2 节、FR-WGT-07）：贴着小组件，从它上方弹出；小组件在屏幕上半部分时改从下方弹出。
// 宽度与小组件一样，高度跟着卡片多少变；没有卡片、或小组件看不见（被隐藏、前台全屏）时把自己藏起来，
// 小组件再出现时跟着出现。显示时不抢焦点（配置里 `focus: false`）。
import { onBeforeUnmount, ref, watch, type Ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  LogicalSize,
  PhysicalPosition,
  Window,
  getCurrentWindow,
  monitorFromPoint,
} from '@tauri-apps/api/window'

/** 卡片层和小组件之间的空隙（逻辑像素） */
const GAP = 8
/** 有卡片时多久看一次小组件（被隐藏、被拖走、回来了） */
const POLL_MS = 1_000

export function useCardsWindow(count: Ref<number>, content: Ref<HTMLElement | null>, topmost: Ref<boolean>) {
  /** 卡片在小组件下方（小组件在屏幕上半部分） */
  const below = ref(false)
  let poll: ReturnType<typeof setInterval> | undefined
  let unlistenMove: UnlistenFn | null = null
  let observer: ResizeObserver | null = null
  let busy = false
  let again = false

  async function place(): Promise<void> {
    // 上一次还没摆完就记一笔，摆完再来一次，免得并发调接口
    if (busy) {
      again = true
      return
    }
    busy = true
    try {
      const me = getCurrentWindow()
      const widget = await Window.getByLabel('widget')
      if (count.value === 0 || !widget || !(await widget.isVisible())) {
        if (await me.isVisible()) await me.hide()
        return
      }
      if (!unlistenMove) unlistenMove = await widget.onMoved(() => void place())
      const [pos, size, scale] = await Promise.all([
        widget.outerPosition(),
        widget.outerSize(),
        me.scaleFactor(),
      ])
      const h = Math.max(1, Math.ceil(content.value?.getBoundingClientRect().height ?? 0))
      const monitor = await monitorFromPoint(pos.x + size.width / 2, pos.y + size.height / 2)
      const work = monitor?.workArea
      const middle = work ? work.position.y + work.size.height / 2 : Number.POSITIVE_INFINITY
      below.value = pos.y + size.height / 2 < middle
      const gap = Math.round(GAP * scale)
      const y = below.value ? pos.y + size.height + gap : pos.y - Math.round(h * scale) - gap
      await me.setSize(new LogicalSize(size.width / scale, h))
      await me.setPosition(new PhysicalPosition(pos.x, y))
      await me.setAlwaysOnTop(topmost.value)
      if (!(await me.isVisible())) await me.show()
    } catch (e) {
      console.warn('摆放卡片层失败', e)
    } finally {
      busy = false
      if (again) {
        again = false
        void place()
      }
    }
  }

  function start(): void {
    watch(count, (n) => {
      clearInterval(poll)
      poll = n > 0 ? setInterval(() => void place(), POLL_MS) : undefined
      void place()
    })
    watch(topmost, () => void place())
    if (typeof ResizeObserver !== 'undefined' && content.value) {
      observer = new ResizeObserver(() => void place())
      observer.observe(content.value)
    }
  }

  onBeforeUnmount(() => {
    clearInterval(poll)
    unlistenMove?.()
    observer?.disconnect()
  })

  return { below, start, place }
}
