// 小组件的自评（04 FR-STA-10、07 FR-WGT-02/03/06）：开关“我现在…”面板、提交、跟踪“你说的”覆盖期。
// 覆盖期内状态行显示“你说的：有点累”，到期或下一次自评时结束；到期由本地定时器收尾，不必等后端事件。
import { onBeforeUnmount, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type SelfWeather } from '@/api'
import { localDate, overrideFrom, overrideFromList, type SelfOverride } from '@/selfReport'

export function useSelfReport(now: () => number = Date.now) {
  const active = ref<SelfOverride | null>(null)
  const panelOpen = ref(false)
  let expiry: ReturnType<typeof setTimeout> | undefined
  let unlisten: UnlistenFn | null = null

  function apply(o: SelfOverride | null): void {
    clearTimeout(expiry)
    active.value = o
    if (o) expiry = setTimeout(() => (active.value = null), o.until - now())
  }

  async function init(): Promise<void> {
    // 先订阅再取当天记录；取记录期间来了事件，以事件为准
    let gotEvent = false
    unlisten = await events.selfReportChanged.listen((e) => {
      gotEvent = true
      apply(overrideFrom(e.payload.weather, e.payload.until_ts, now()))
    })
    const items = await unwrap(commands.selfReportList(localDate(new Date(now()))))
    if (!gotEvent) apply(overrideFromList(items, now()))
  }

  /** 提交后关面板；状态行等 self_report:changed 再变（后端 1 秒内推送，FR-STA-10 验收）。 */
  async function submit(weather: SelfWeather, note: string): Promise<void> {
    const trimmed = note.trim()
    await unwrap(commands.selfReportSet(weather, trimmed === '' ? null : trimmed))
    panelOpen.value = false
  }

  onBeforeUnmount(() => {
    unlisten?.()
    clearTimeout(expiry)
  })

  return { active, panelOpen, init, submit }
}
