// 单独同意项（07 FR-ONB-04、09 第 2.1 节）的编号，引导页与设置“隐私”共用。
import type { ConsentItem } from '@/api'

export const CONSENT_NUMBER: Record<ConsentItem, string> = {
  sense: '①',
  jev_features: '②',
  schedule: '③',
  llm_summary: '④',
  enhanced: '⑤',
  rewrite: '⑥',
}

/** 会把数据发给第三方服务的同意项：撤回时如实说明已发出的收不回来（FR-DAT-05）。① 只在本机，不用说明。 */
export function sendsToThirdParty(item: ConsentItem): boolean {
  return item !== 'sense'
}
