// 小组件右键菜单（07 FR-WGT-06）。菜单项按规格顺序排列：
// 我现在… · 开始/结束专注 · 换装 · 暂停/恢复感知 · 打开看板 · 待确认日程与待办 · 设置 · 隐藏小组件。
// 后端还没有的功能（自评 B-07、专注 B-08、换装 P2、待确认 C-05）暂不出现，随对应任务按上面的顺序插进来。
import type { StatusSnapshot } from '@/api'
import { t } from '@/i18n'

export type MenuAction = 'pause' | 'resume' | 'dashboard' | 'settings' | 'hide'

export interface MenuEntry {
  id: MenuAction
  text: string
}

export function menuEntries(s: StatusSnapshot | null): MenuEntry[] {
  const pause: MenuAction = s?.paused ? 'resume' : 'pause'
  const ids: MenuAction[] = [pause, 'dashboard', 'settings', 'hide']
  return ids.map((id) => ({ id, text: t(`widget.menu.${id}`) }))
}
