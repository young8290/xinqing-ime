// Hub 设置缓存（10 第 6.2 节）：只缓存后端的值，变化以 `settings:changed` 为准。
import { defineStore } from 'pinia'
import { reactive } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type SettingValue } from '@/api'

export const useSettingsStore = defineStore('settings', () => {
  const values = reactive<Record<string, SettingValue>>({})
  let unlisten: UnlistenFn | null = null

  async function load(key: string): Promise<SettingValue> {
    const v = await unwrap(commands.settingsGet(key))
    values[key] = v
    return v
  }

  /** 读入本窗口关心的键，并订阅变化（别的窗口改了设置，这里自动刷新）。 */
  async function init(keys: string[]): Promise<void> {
    if (!unlisten) {
      unlisten = await events.settingsChanged.listen((e) => {
        if (e.payload.key in values) void load(e.payload.key)
      })
    }
    await Promise.all(keys.map(load))
  }

  async function set(key: string, value: SettingValue): Promise<void> {
    await unwrap(commands.settingsSet(key, value))
    values[key] = value
  }

  function dispose(): void {
    unlisten?.()
    unlisten = null
  }

  return { values, init, load, set, dispose }
})
