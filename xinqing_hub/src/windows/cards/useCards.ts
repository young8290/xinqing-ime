// 卡片层的事件与按钮（07 FR-WGT-07）：后端事件 → 卡片进队；按钮 → 命令，做完收起。
// 日程与待办卡片 60 秒没操作就收起，留在“待确认”列表里（FR-SCH-05 第 3 条）；鼠标停在卡片上时不收。
// 错过的提醒在 Hub 启动那一刻推送，这个窗口可能还没加载完：打开时、收到事件时都去取一次，后端只给一次（ADR 0036）。
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type RestAction, type RestKind } from '@/api'
import { errorText } from '@/i18n'
import { add, remove, update, visible, type Card, type Queued } from './queue'
import { emitCards, emitReviewHint, openDashboard } from '../shared/bus'

/** 日程与待办卡片无操作多久收起（FR-SCH-05 第 3 条） */
export const COLLAPSE_MS = 60_000
/** 自评后的“和晴晴聊聊”留多久 */
export const SELF_REPLY_MS = 10 * 60_000
/** “已添加 ✓”停留多久 */
export const DONE_MS = 1_500

export function useCards() {
  const q = ref<Queued[]>([])
  const view = computed(() => visible(q.value))
  const count = computed(() => q.value.length)
  /** 某张卡片上的出错提示（DS-COPY-06），下次操作时清掉 */
  const errors = ref<Record<string, string>>({})
  /** 鼠标停在卡片上：到点也先不收 */
  const hovering = ref(false)
  let seq = 0
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  const unlisten: UnlistenFn[] = []

  function push(card: Card): void {
    q.value = add(q.value, card, ++seq)
  }

  function drop(key: string): void {
    clearTimeout(timers.get(key))
    timers.delete(key)
    delete errors.value[key]
    q.value = remove(q.value, key)
  }

  function patch(key: string, fn: (c: Card) => Card): void {
    q.value = update(q.value, key, fn)
  }

  function collapseLater(key: string, ms: number): void {
    clearTimeout(timers.get(key))
    timers.set(
      key,
      setTimeout(() => {
        if (hovering.value) collapseLater(key, 10_000)
        else drop(key)
      }, ms),
    )
  }

  async function takeMissed(): Promise<void> {
    const missed = await commands.reminderMissedTake()
    if (missed && missed.items.length > 0) push({ key: 'missed', kind: 'missed', missed })
  }

  async function init(): Promise<void> {
    unlisten.push(
      await events.scheduleDetected.listen((e) => {
        const key = `schedule:${e.payload.card_id}`
        push({ key, kind: 'schedule', cardId: e.payload.card_id, item: null, conflicts: [], done: false })
        // 识别一直没结果也不能一直挂着
        collapseLater(key, COLLAPSE_MS)
      }),
      await events.scheduleReady.listen((e) => {
        const key = `schedule:${e.payload.card_id}`
        // 24 小时内提示过同一件事（FR-SCH-06）：卡片直接收起
        if (!e.payload.schedule) return drop(key)
        push({
          key,
          kind: 'schedule',
          cardId: e.payload.card_id,
          item: e.payload.schedule,
          conflicts: e.payload.conflicts,
          done: false,
        })
        collapseLater(key, COLLAPSE_MS)
      }),
      await events.todoDetected.listen((e) => {
        const key = `todo:${e.payload.card_id}`
        push({ key, kind: 'todo', cardId: e.payload.card_id, item: null, done: false })
        collapseLater(key, COLLAPSE_MS)
      }),
      await events.todoReady.listen((e) => {
        const key = `todo:${e.payload.card_id}`
        if (!e.payload.todo) return drop(key)
        push({ key, kind: 'todo', cardId: e.payload.card_id, item: e.payload.todo, done: false })
        collapseLater(key, COLLAPSE_MS)
      }),
      await events.reminderDue.listen((e) =>
        push({ key: `reminder:${e.payload.id}`, kind: 'reminder', due: e.payload }),
      ),
      await events.reminderMissed.listen(
        () => void takeMissed().catch((e) => console.warn('取错过的提醒失败', e)),
      ),
      await events.restDue.listen((e) => push({ key: 'rest', kind: 'rest', due: e.payload })),
      await events.comfortNew.listen((e) => {
        if (e.payload.trigger !== 'self_report') return
        push({ key: 'self_reply', kind: 'self_reply', comfortId: e.payload.id })
        collapseLater('self_reply', SELF_REPLY_MS)
      }),
      await events.reviewEvening.listen((e) => push({ key: 'evening', kind: 'evening', summary: e.payload })),
      await events.letterNew.listen((e) =>
        push({ key: `letter:${e.payload.id}`, kind: 'letter', letter: e.payload }),
      ),
    )
    await takeMissed()
  }

  /** 做一件事，成功就按 `then` 收尾；失败把提示写在这张卡片上，卡片留着可以重试。 */
  async function act(
    key: string,
    action: () => Promise<unknown>,
    then: () => void = () => drop(key),
  ): Promise<void> {
    delete errors.value[key]
    try {
      await action()
      then()
    } catch (e) {
      errors.value = { ...errors.value, [key]: errorText(e) }
    }
  }

  const actions = {
    /** 日程：添加后显示“已添加 ✓”再收起（FR-SCH-05） */
    scheduleAdd: (key: string, id: number) =>
      act(
        key,
        () => unwrap(commands.scheduleConfirm(id)),
        () => {
          patch(key, (c) => (c.kind === 'schedule' ? { ...c, done: true } : c))
          collapseLater(key, DONE_MS)
        },
      ),
    /** “修改”：在看板“日程与待办”页打开编辑框（卡片只有 320 宽，放不下完整的表单） */
    scheduleEdit: (key: string, id: number) =>
      act(key, () => openDashboard({ page: 'schedule', edit: String(id) })),
    scheduleIgnore: (key: string, id: number) => act(key, () => unwrap(commands.scheduleIgnore(id))),
    todoAdd: (key: string, id: number) =>
      act(
        key,
        () => unwrap(commands.todoConfirm(id)),
        () => {
          patch(key, (c) => (c.kind === 'todo' ? { ...c, done: true } : c))
          collapseLater(key, DONE_MS)
        },
      ),
    todoEdit: (key: string, id: number) =>
      act(key, () => openDashboard({ page: 'schedule', edit: `todo:${id}` })),
    todoIgnore: (key: string, id: number) => act(key, () => unwrap(commands.todoIgnore(id))),
    /** 到点提醒：知道了 / 5 / 10 分钟后（FR-SCH-07） */
    reminder: (key: string, id: number, action: string) =>
      act(key, () => unwrap(commands.reminderAction(id, action))),
    openSchedule: (key: string) => act(key, () => openDashboard({ page: 'schedule' })),
    rest: (kind: RestKind, action: RestAction) =>
      act('rest', () => unwrap(commands.restAction(kind, action))),
    chat: (key: string) => act(key, () => unwrap(commands.openWindow('chat'))),
    /** 晚间小结的“看看今天的看板” */
    openToday: (key: string) => act(key, () => openDashboard({ page: 'today' })),
    openLetter: (key: string, id: number) => act(key, () => openDashboard({ page: 'mailbox', letter: id })),
    /** 周信“等会儿再看”：一句话区留一条提示，点了也能打开（FR-WGT-04） */
    letterLater: (key: string, id: number) => act(key, () => emitReviewHint({ letter: id })),
    dismiss: drop,
  }

  // 告诉小组件现在有没有卡片：有卡片时小组件不贴边收起
  watch(count, (n) => void emitCards({ count: n }).catch((e) => console.warn('通知小组件失败', e)))

  onBeforeUnmount(() => {
    for (const t of timers.values()) clearTimeout(t)
    for (const u of unlisten) u()
  })

  return { view, count, errors, hovering, init, actions }
}
