// 悬停（或键盘聚焦）状态行时显示状态解释（07 FR-WGT-06、04 FR-STA-09）。
// 每次打开都重新取 state_explain：解释只在状态切换时更新，取一次很便宜，也不会拿到过期的缓存。
import { ref, watch, type Ref } from 'vue'
import { commands, unwrap, type StatusSnapshot } from '@/api'
import { explainHeader, explainLines, type ExplainLines } from '@/explain'

/** 鼠标停留这么久才打开，扫过状态行时不闪 */
const OPEN_DELAY_MS = 300
/** 从状态行移到面板上（或反过来）的间隙不算离开 */
const CLOSE_DELAY_MS = 150

/**
 * 面板内容。暂停、未连接时没有解释；后端的解释和当前快照不是同一个状态（刚切换、还没缓存）时不用它；
 * 启动后还没切换过、没有解释时，有可能性就只显示标题，否则不显示。
 */
export async function loadExplain(s: StatusSnapshot | null): Promise<ExplainLines | null> {
  if (!s || s.paused || !s.connected) return null
  try {
    const e = await unwrap(commands.stateExplain(null))
    if (e && e.state === s.state) return explainLines(e)
  } catch {
    // 取不到解释时退回只显示标题，不打扰用户
  }
  if (s.prob === null) return null
  return { header: explainHeader(s.state, Math.round(s.prob * 100)), signals: [], footer: '' }
}

export function useExplain(snapshot: Ref<StatusSnapshot | null>) {
  const lines = ref<ExplainLines | null>(null)
  let openTimer: ReturnType<typeof setTimeout> | undefined
  let closeTimer: ReturnType<typeof setTimeout> | undefined
  let seq = 0

  async function open(): Promise<void> {
    clearTimeout(openTimer)
    clearTimeout(closeTimer)
    const mine = ++seq
    const l = await loadExplain(snapshot.value)
    // 等结果的时候已经移开或又打开了一次，以最后一次为准
    if (mine === seq) lines.value = l
  }

  /** 鼠标移入：稍等再打开；已经开着就保持 */
  function hover(): void {
    clearTimeout(closeTimer)
    if (lines.value) return
    clearTimeout(openTimer)
    openTimer = setTimeout(() => void open(), OPEN_DELAY_MS)
  }

  /** 鼠标移出：稍等再关，移到面板上会被 hover() 取消 */
  function leave(): void {
    clearTimeout(openTimer)
    clearTimeout(closeTimer)
    closeTimer = setTimeout(close, CLOSE_DELAY_MS)
  }

  function close(): void {
    clearTimeout(openTimer)
    clearTimeout(closeTimer)
    seq++
    lines.value = null
  }

  // 状态变了，开着的解释就过期了（每个事件都会换一份快照，所以逐项比较，可能性变了不算）
  watch([() => snapshot.value?.state, () => snapshot.value?.paused, () => snapshot.value?.connected], () =>
    close(),
  )

  return { lines, open, hover, leave, close }
}
