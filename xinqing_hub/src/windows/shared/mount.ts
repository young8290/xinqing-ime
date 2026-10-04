// 每个窗口入口的公共启动步骤：Pinia、主题（DS-COLOR-03）、不可见时暂停动画、窗口标题、挂载。
import '@/styles/tokens.css'
import '@/styles/base.css'
import { createApp, watchEffect, type Component } from 'vue'
import { createPinia } from 'pinia'
import { t, type CopyKey } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'

export function applyTheme(theme: unknown): void {
  const root = document.documentElement
  if (theme === 'light' || theme === 'dark') root.dataset.theme = theme
  else delete root.dataset.theme
}

/** DS-MOTION-05：窗口不可见时暂停全部动画（base.css 按 data-hidden 生效）。 */
export function trackVisibility(): void {
  const sync = () => {
    if (document.hidden) document.documentElement.dataset.hidden = ''
    else delete document.documentElement.dataset.hidden
  }
  document.addEventListener('visibilitychange', sync)
  sync()
}

export async function mountWindow(app: Component, title: CopyKey): Promise<void> {
  document.title = t(title)
  trackVisibility()
  const vue = createApp(app)
  vue.use(createPinia())

  const settings = useSettingsStore()
  try {
    await settings.init(['ui.theme'])
  } catch (e) {
    // 在普通浏览器里预览界面（pnpm dev）时没有后端，按默认值继续
    console.warn('读取设置失败，使用默认主题', e)
  }
  watchEffect(() => applyTheme(settings.values['ui.theme']))

  vue.mount('#app')
}
