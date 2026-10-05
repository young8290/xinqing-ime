// 小组件的休息提醒卡片（06 FR-RST-02～06）：收到 rest:due 显示卡片，按钮结果交回后端（rest_action）。
// 护眼点“已完成”先走 20 秒倒计时，结束后显示“眼睛说谢谢你”再收起；其余几类点了就收起。
import { onBeforeUnmount, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type RestAction, type RestDue, type RestKind } from '@/api'
import type { CopyKey } from '@/i18n'

/** 护眼倒计时秒数、致谢停留毫秒（FR-RST-02）。 */
export const EYE_SECONDS = 20
export const THANKS_MS = 2000

/** 卡片正文的文案键。 */
export function restText(kind: RestKind, tired: boolean): CopyKey {
  if (kind === 'eye' && tired) return 'rest.eye_tired'
  return `rest.${kind}`
}

/** “已完成”按钮的文案：喝水卡片叫“喝了”（FR-RST-03）。 */
export function doneLabel(kind: RestKind): CopyKey {
  return kind === 'water' ? 'rest.btn_drank' : 'rest.btn_done'
}

export function useRest() {
  const due = ref<RestDue | null>(null)
  /** 护眼倒计时剩余秒数；没在倒计时为 null，0 表示倒计时结束、正在显示致谢 */
  const countdown = ref<number | null>(null)
  let timer: ReturnType<typeof setInterval> | undefined
  let unlisten: UnlistenFn | null = null

  function stopTimer(): void {
    clearInterval(timer)
    timer = undefined
  }

  async function init(): Promise<void> {
    unlisten = await events.restDue.listen((e) => {
      stopTimer()
      countdown.value = null
      due.value = e.payload
    })
  }

  async function act(action: RestAction): Promise<void> {
    const current = due.value
    if (!current) return
    if (action === 'done' && current.kind === 'eye' && countdown.value === null) {
      countdown.value = EYE_SECONDS
      timer = setInterval(() => {
        if (countdown.value === null) return
        countdown.value -= 1
        if (countdown.value > 0) return
        stopTimer()
        setTimeout(() => void finish(current.kind, 'done'), THANKS_MS)
      }, 1000)
      return
    }
    await finish(current.kind, action)
  }

  async function finish(kind: RestKind, action: RestAction): Promise<void> {
    stopTimer()
    if (due.value?.kind === kind) {
      due.value = null
      countdown.value = null
    }
    await unwrap(commands.restAction(kind, action))
  }

  onBeforeUnmount(() => {
    unlisten?.()
    stopTimer()
  })

  return { due, countdown, init, act }
}
