// 小组件右键菜单（07 FR-WGT-06）。菜单项按规格顺序排列：
// 我现在… · 开始/结束专注 · 换装 · 暂停/恢复感知 · 打开看板 · 待确认日程与待办 · 设置 · 隐藏小组件。
// 后端还没有的功能（专注 FR-RST-10、换装 FR-WGT-08，都是 P2）暂不出现，随对应任务按上面的顺序插进来。
// “待确认日程与待办”带上数量（FR-ENT-01 同一写法：为 0 时不显示数字），点了打开看板“日程与待办”页。
import type { StatusSnapshot } from '@/api'
import { t } from '@/i18n'

export type MenuAction = 'self_report' | 'pause' | 'resume' | 'dashboard' | 'pending' | 'settings' | 'hide'

export interface MenuEntry {
  id: MenuAction
  text: string
}

export function menuEntries(s: StatusSnapshot | null, pending = 0): MenuEntry[] {
  const pause: MenuAction = s?.paused ? 'resume' : 'pause'
  const ids: MenuAction[] = ['self_report', pause, 'dashboard', 'pending', 'settings', 'hide']
  // “我现在…”与状态行的 ✎ 共用一条文案
  const text = (id: MenuAction) => {
    if (id === 'self_report') return t('widget.self_report_prompt')
    if (id === 'pending')
      return pending > 0 ? t('widget.menu.pending_n', { n: pending }) : t('widget.menu.pending')
    return t(`widget.menu.${id}`)
  }
  return ids.map((id) => ({ id, text: text(id) }))
}
