// 首次引导交给小组件的两件事（07 FR-ONB-05）：小组件放在哪个角、要不要打招呼。
// 都是本机界面偏好，和小组件位置一样放 WebView 的 localStorage（各窗口同源共享）；读写失败就当没设过。

/** 小组件默认放在主显示器工作区的哪个角 */
export type Corner = 'br' | 'tr' | 'bl' | 'tl'
export const CORNERS: readonly Corner[] = ['br', 'tr', 'bl', 'tl']

const CORNER_KEY = 'xq.widget.corner'
const GREET_KEY = 'xq.widget.greet'
/** 小组件记住的位置（useWidgetWindow.ts 的 STORAGE_KEY） */
const PLACEMENT_KEY = 'xq.widget.placement'

export function loadCorner(): Corner {
  try {
    const v = localStorage.getItem(CORNER_KEY)
    return (CORNERS as readonly string[]).includes(v ?? '') ? (v as Corner) : 'br'
  } catch {
    return 'br'
  }
}

/** 记下用户选的角，并忘掉之前记住的位置，下次显示小组件就放到这个角。 */
export function saveCorner(c: Corner): void {
  try {
    localStorage.setItem(CORNER_KEY, c)
    localStorage.removeItem(PLACEMENT_KEY)
  } catch {
    // 存不下就用默认的右下角
  }
}

/** 引导完成时调用：小组件下次打开时由晴晴打招呼。 */
export function requestGreeting(): void {
  try {
    localStorage.setItem(GREET_KEY, '1')
  } catch {
    // 不打招呼也不影响使用
  }
}

/** 小组件打开时调用：有招呼要打就返回 true，并且只打这一次。 */
export function takeGreeting(): boolean {
  try {
    const v = localStorage.getItem(GREET_KEY) === '1'
    localStorage.removeItem(GREET_KEY)
    return v
  } catch {
    return false
  }
}
