// 卡片层的队列（07 FR-WGT-07）：最多同时显示 2 张，更多的排队；高优先级的先显示（FR-WGT-04 的顺序）：
//   自评回应 > 日程与待办（含到点提醒、错过的提醒）> 休息提醒 > 晚间小结 / 周信提示。
// 同一优先级先来先显示。纯函数，事件与按钮在 useCards.ts。
import type {
  ConflictItem,
  EveningSummary,
  LetterNew,
  ReminderDue,
  ReminderMissed,
  RestDue,
  ScheduleItem,
  TodoItem,
} from '@/api'

export type Card =
  /** 识别出的日程：`item` 为 null 时是“识别中…”；`done` 是点了“添加”后的“已添加 ✓” */
  | {
      key: string
      kind: 'schedule'
      cardId: number
      item: ScheduleItem | null
      conflicts: ConflictItem[]
      done: boolean
    }
  | { key: string; kind: 'todo'; cardId: number; item: TodoItem | null; done: boolean }
  /** 日程或待办到点（FR-SCH-07、FR-SCH-14） */
  | { key: string; kind: 'reminder'; due: ReminderDue }
  /** Hub 启动时 12 小时内错过的提醒，集中一张（FR-SCH-07） */
  | { key: string; kind: 'missed'; missed: ReminderMissed }
  | { key: string; kind: 'rest'; due: RestDue }
  /** 负面自评后的“和晴晴聊聊”（FR-STA-10 第 2 条）；晴晴的回应在小组件一句话区 */
  | { key: string; kind: 'self_reply'; comfortId: number }
  | { key: string; kind: 'evening'; summary: EveningSummary }
  | { key: string; kind: 'letter'; letter: LetterNew }

export type CardKind = Card['kind']

export const MAX_VISIBLE = 2

/** 数字越小越优先 */
export const RANK: Record<CardKind, number> = {
  self_reply: 0,
  schedule: 1,
  todo: 1,
  reminder: 1,
  missed: 1,
  rest: 2,
  evening: 3,
  letter: 3,
}

export interface Queued {
  card: Card
  /** 进队的先后 */
  seq: number
}

/** 放进一张卡片；同一个 key 的旧卡片被替换（位置按新的算，休息提醒只留最新一张）。 */
export function add(q: readonly Queued[], card: Card, seq: number): Queued[] {
  return [...q.filter((x) => x.card.key !== card.key), { card, seq }]
}

/** 改一张卡片（识别完成填字段、点了“添加”变成“已添加 ✓”）；找不到时原样返回。 */
export function update(q: readonly Queued[], key: string, fn: (c: Card) => Card): Queued[] {
  return q.map((x) => (x.card.key === key ? { ...x, card: fn(x.card) } : x))
}

export function remove(q: readonly Queued[], key: string): Queued[] {
  return q.filter((x) => x.card.key !== key)
}

/** 排好序的全部卡片 */
export function ordered(q: readonly Queued[]): Card[] {
  return [...q].sort((a, b) => RANK[a.card.kind] - RANK[b.card.kind] || a.seq - b.seq).map((x) => x.card)
}

/** 现在显示的（最多 2 张）和还在排队的张数 */
export function visible(q: readonly Queued[]): { shown: Card[]; waiting: number } {
  const all = ordered(q)
  return { shown: all.slice(0, MAX_VISIBLE), waiting: Math.max(0, all.length - MAX_VISIBLE) }
}
