// 休息提醒卡片（06 FR-RST-02～06）的文案选择与时长：收到 rest:due 显示卡片，按钮结果交回后端（rest_action）。
// 护眼点“已完成”先走 20 秒倒计时，结束后显示“眼睛说谢谢你”再收起（倒计时在 RestCard.vue）；其余几类点了就收起。
import type { RestKind } from '@/api'
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
