// 小组件一句话区的消息队列（07 FR-WGT-04）：高优先级的盖住低优先级的，低优先级的留着等，过期的自己消失。
// 卡片类消息（日程与待办、休息提醒、晚间小结、周信）在卡片层（FR-WGT-07），这里只放文字类：
//   求助入口 > 出错提示 > 暖心话 > 提示（降档、数据重建、首次招呼）> 周信提示 > 今日一句 > 空闲问候。
// 每种消息同一时间只留一条，新来的替换旧的。纯函数，计时器和事件在 useMessages.ts。

export type MessageKind = 'safety' | 'error' | 'comfort' | 'notice' | 'review' | 'today' | 'idle'

/** 数字越小越优先 */
export const RANK: Record<MessageKind, number> = {
  safety: 0,
  error: 1,
  comfort: 2,
  notice: 3,
  review: 4,
  today: 5,
  idle: 6,
}

/** 点一下这条消息做什么 */
export type MessageAction = { type: 'safety' } | { type: 'letter'; id: number }

export interface Message {
  kind: MessageKind
  text: string
  /** 大模型写的：句尾显示 `AI 生成`（FR-CMF-04 第 2 条） */
  ai: boolean
  /** 暖心话的行号：悬停时可以反馈（FR-CMF-05）；不是暖心话为 null */
  comfortId: number | null
  action: MessageAction | null
  /** 什么时候出现的（Unix 毫秒） */
  at: number
  /** 到这一刻就不再显示；null 表示一直在 */
  until: number | null
}

/** 暖心话 2 小时后淡出为“今日一句”（FR-CMF-04 第 3 条） */
export const COMFORT_MS = 2 * 60 * 60 * 1000
/** 出错提示停留多久 */
export const ERROR_MS = 8_000
/** 降档、数据重建这类提示停留多久 */
export const NOTICE_MS = 10 * 60 * 1000
/** 求助入口最多留多久（点了就收起；一直没点也不能永远挂着） */
export const SAFETY_MS = 2 * 60 * 60 * 1000

export function message(kind: MessageKind, text: string, at: number, extra: Partial<Message> = {}): Message {
  return { kind, text, ai: false, comfortId: null, action: null, at, until: null, ...extra }
}

/** 放进一条消息：同种的旧消息被替换。 */
export function put(list: readonly Message[], m: Message): Message[] {
  return [...list.filter((x) => x.kind !== m.kind), m]
}

/** 去掉一种消息（点过的求助入口、看过的周信提示）。 */
export function drop(list: readonly Message[], kind: MessageKind): Message[] {
  return list.filter((x) => x.kind !== kind)
}

/** 去掉已经过期的。 */
export function prune(list: readonly Message[], now: number): Message[] {
  return list.filter((x) => x.until === null || x.until > now)
}

/** 现在该显示哪条：没过期的里面优先级最高的。 */
export function current(list: readonly Message[], now: number): Message | null {
  let best: Message | null = null
  for (const m of prune(list, now)) {
    if (!best || RANK[m.kind] < RANK[best.kind]) best = m
  }
  return best
}

/** 下一次需要重新挑选的时刻（最早的过期时间）；没有会过期的为 null。 */
export function nextChange(list: readonly Message[], now: number): number | null {
  const times = list.map((m) => m.until).filter((u): u is number => u !== null && u > now)
  return times.length ? Math.min(...times) : null
}

/** 当天结束的时刻（今日一句、周信提示到这一刻为止）。 */
export function endOfDay(now: number): number {
  const d = new Date(now)
  d.setHours(24, 0, 0, 0)
  return d.getTime()
}

/**
 * 一句暖心话：先以“暖心话”显示 2 小时，同时备一条同样文字的“今日一句”到当天结束，
 * 暖心话过期后自然露出今日一句（FR-CMF-04 第 3 条、FR-WGT-04）。
 */
export function putComfort(
  list: readonly Message[],
  c: { id: number; text: string; ai: boolean },
  now: number,
): Message[] {
  const common = { ai: c.ai, comfortId: c.id }
  const withComfort = put(list, message('comfort', c.text, now, { ...common, until: now + COMFORT_MS }))
  return put(withComfort, message('today', c.text, now, { ...common, until: endOfDay(now) }))
}
