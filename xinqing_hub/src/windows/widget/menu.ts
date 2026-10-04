// 小组件右键菜单（07 FR-WGT-06）。菜单项按规格顺序排列：
// 我现在… · 开始/结束专注 · 换装 · 暂停/恢复感知 · 打开看板 · 待确认日程与待办 · 设置 · 隐藏小组件。
// 后端还没有的功能（专注 B-08、换装 P2、待确认 C-05）暂不出现，随对应任务按上面的顺序插进来。
import type { StatusSnapshot } from '@/api'
import { t } from '@/i18n'

export type MenuAction = 'self_report' | 'pause' | 'resume' | 'dashboard' | 'settings' | 'hide'

export interface MenuEntry {
  id: MenuAction
  text: string
}

export function menuEntries(s: StatusSnapshot | null): MenuEntry[] {
  const pause: MenuAction = s?.paused ? 'resume' : 'pause'
  const ids: MenuAction[] = ['self_report', pause, 'dashboard', 'settings', 'hide']
  // “我现在…”与状态行的 ✎ 共用一条文案
  const text = (id: MenuAction) =>
    id === 'self_report' ? t('widget.self_report_prompt') : t(`widget.menu.${id}`)
  return ids.map((id) => ({ id, text: text(id) }))
}
