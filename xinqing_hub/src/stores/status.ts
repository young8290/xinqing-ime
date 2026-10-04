// 显示状态快照（17 第 3.2 节）：打开窗口时取一次快照，之后只跟随 `status:changed`。
import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type StatusSnapshot } from '@/api'

export const useStatusStore = defineStore('status', () => {
  const snapshot = ref<StatusSnapshot | null>(null)
  let unlisten: UnlistenFn | null = null

  async function init(): Promise<void> {
    if (unlisten) return
    // 先订阅再取快照；取快照期间若已收到事件，以事件为准（事件一定比这次快照新）
    let gotEvent = false
    unlisten = await events.statusChanged.listen((e) => {
      gotEvent = true
      snapshot.value = e.payload
    })
    const first = await unwrap(commands.getStatus())
    if (!gotEvent) snapshot.value = first
  }

  async function setPaused(on: boolean): Promise<void> {
    await unwrap(commands.pauseSet(on))
  }

  function dispose(): void {
    unlisten?.()
    unlisten = null
  }

  return { snapshot, init, setPaused, dispose }
})
