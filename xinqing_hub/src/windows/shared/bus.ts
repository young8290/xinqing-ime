// 窗口之间的界面事件（只在前端之间传，不经后端，不是 bindings.ts 的契约）：
// - 卡片层告诉小组件现在有几张卡片（小组件据此不贴边收起）；
// - 周信卡片点了“等会儿再看”，小组件一句话区留一条提示（FR-WGT-04）；
// - 别的窗口要看板打开某一页（底栏“下一个日程”、卡片上的“修改”“打开看看”等）。
// 都用全局广播 emit：每个窗口只听自己关心的，名字统一加 `xq:` 前缀，与后端事件区分。
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { commands, unwrap } from '@/api'

export const CARDS_EVENT = 'xq:cards'
export const REVIEW_HINT_EVENT = 'xq:review-hint'
export const DASHBOARD_NAV_EVENT = 'xq:dashboard-nav'

/** 看板的页 */
export type DashboardPage = 'today' | 'calendar' | 'weekly' | 'schedule' | 'mailbox' | 'chat_diary'

/** 打开看板时要定位到的东西 */
export interface DashboardNav {
  page: DashboardPage
  /** 日程页：打开这条日程（或 `todo:` 前缀的待办）的编辑框 */
  edit?: string
  /** 信箱：打开这封信 */
  letter?: number
}

/** 卡片层给小组件的：现在有几张卡片 */
export interface CardsState {
  count: number
}

/** 周信的一句话区提示 */
export interface ReviewHint {
  letter: number
}

/** 看板刚打开时还没开始听事件：要去的页同时写进 localStorage，看板挂载时取一次。 */
export const NAV_STORAGE_KEY = 'xq.dashboard.nav'

export async function openDashboard(nav: DashboardNav): Promise<void> {
  try {
    localStorage.setItem(NAV_STORAGE_KEY, JSON.stringify(nav))
  } catch {
    // 存不下时看板照常打开，只是停在上次的页
  }
  await unwrap(commands.openWindow('dashboard'))
  await emit(DASHBOARD_NAV_EVENT, nav)
}

/** 看板挂载时取走 localStorage 里的导航（取过就删，免得下次打开又跳过去）。 */
export function takeDashboardNav(): DashboardNav | null {
  try {
    const raw = localStorage.getItem(NAV_STORAGE_KEY)
    localStorage.removeItem(NAV_STORAGE_KEY)
    const v: unknown = raw ? JSON.parse(raw) : null
    return v && typeof v === 'object' && 'page' in v ? (v as DashboardNav) : null
  } catch {
    return null
  }
}

export function onDashboardNav(cb: (nav: DashboardNav) => void): Promise<UnlistenFn> {
  return listen<DashboardNav>(DASHBOARD_NAV_EVENT, (e) => {
    try {
      localStorage.removeItem(NAV_STORAGE_KEY)
    } catch {
      // 读写失败不影响跳页
    }
    cb(e.payload)
  })
}

export function emitCards(state: CardsState): Promise<void> {
  return emit(CARDS_EVENT, state)
}

export function onCards(cb: (state: CardsState) => void): Promise<UnlistenFn> {
  return listen<CardsState>(CARDS_EVENT, (e) => cb(e.payload))
}

export function emitReviewHint(hint: ReviewHint): Promise<void> {
  return emit(REVIEW_HINT_EVENT, hint)
}

export function onReviewHint(cb: (hint: ReviewHint) => void): Promise<UnlistenFn> {
  return listen<ReviewHint>(REVIEW_HINT_EVENT, (e) => cb(e.payload))
}
