// “陪我呼吸 1 分钟”的节奏（05 FR-CHT-06）：吸 4 秒、停 4 秒、呼 6 秒，共 4 轮。
export const PHASES = [
  { key: 'in', seconds: 4 },
  { key: 'hold', seconds: 4 },
  { key: 'out', seconds: 6 },
] as const

export const ROUNDS = 4

/** 一共多少秒（4 ×（4 + 4 + 6）= 56 秒）。 */
export const TOTAL_SECONDS = ROUNDS * PHASES.reduce((sum, p) => sum + p.seconds, 0)
