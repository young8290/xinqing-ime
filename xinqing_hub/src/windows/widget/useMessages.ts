// 一句话区（07 FR-WGT-04）：把各处来的消息放进队列（messages.ts），到期自动换下一条。
// 来源：暖心话 comfort:new（含自评回应，FR-CMF-04、FR-STA-10）、主动关怀降档 care:reduced、改写的求助入口 safety:invite
// （ADR 0028）、卡片层转来的周信提示、启动时的数据重建提示（FR-DAT-01）、操作出错；窗口打开时用当天最后一句暖心话做“今日一句”。
import { computed, onBeforeUnmount, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap, type ComfortNew } from '@/api'
import { localDate } from '@/format'
import { errorText, t } from '@/i18n'
import {
  ERROR_MS,
  NOTICE_MS,
  SAFETY_MS,
  current,
  drop,
  endOfDay,
  message,
  nextChange,
  prune,
  put,
  putComfort,
  type Message,
  type MessageKind,
} from './messages'
import { onReviewHint } from '../shared/bus'

export function useMessages(now: () => number = Date.now) {
  const list = ref<Message[]>([message('idle', t('greeting.idle'), now())])
  // 只为到期时触发重新计算
  const tick = ref(0)
  const shown = computed(() => {
    void tick.value
    return current(list.value, now())
  })
  let timer: ReturnType<typeof setTimeout> | undefined
  const unlisten: UnlistenFn[] = []

  function schedule(): void {
    clearTimeout(timer)
    const at = nextChange(list.value, now())
    if (at === null) return
    timer = setTimeout(() => {
      list.value = prune(list.value, now())
      tick.value += 1
      schedule()
    }, at - now())
  }

  function set(next: Message[]): void {
    list.value = next
    tick.value += 1
    schedule()
  }

  /** 操作失败：说清发生了什么（DS-COPY-06），几秒后让回原来的消息 */
  function error(e: unknown): void {
    set(put(list.value, message('error', errorText(e), now(), { until: now() + ERROR_MS })))
  }

  function notice(text: string, ms = NOTICE_MS): void {
    set(put(list.value, message('notice', text, now(), { until: now() + ms })))
  }

  function comfort(c: { id: number; text: string; ai: boolean }): void {
    set(putComfort(list.value, c, now()))
  }

  /** 点过的消息收起（求助入口、周信提示） */
  function dismiss(kind: MessageKind): void {
    set(drop(list.value, kind))
  }

  /**
   * 订阅事件并接上“今日一句”。`onComfort` 在新暖心话到来时调用（小精灵“靠近”）。
   * 取今日一句失败只是少一句话，不打扰用户。
   */
  async function init(onComfort: (c: ComfortNew) => void = () => {}): Promise<void> {
    let gotComfort = false
    unlisten.push(
      await events.comfortNew.listen((e) => {
        gotComfort = true
        comfort({ id: e.payload.id, text: e.payload.text, ai: e.payload.ai_generated })
        onComfort(e.payload)
      }),
      await events.careReduced.listen(() => notice(t('widget.care_reduced'))),
      // 改写时命中危机词表：一句话区给最高优先级的求助入口，不自动弹对话窗口（ADR 0028 第 4 条）
      await events.safetyInvite.listen(() =>
        set(
          put(
            list.value,
            message('safety', t('rewrite.crisis_bubble'), now(), {
              action: { type: 'safety' },
              until: now() + SAFETY_MS,
            }),
          ),
        ),
      ),
      await onReviewHint((h) =>
        set(
          put(
            list.value,
            message('review', t('widget.letter_hint'), now(), {
              action: { type: 'letter', id: h.letter },
              until: endOfDay(now()),
            }),
          ),
        ),
      ),
    )
    const [items, rebuilt] = await Promise.all([
      unwrap(commands.comfortList(localDate(new Date(now())))),
      commands.dbRebuiltTake(),
    ])
    const last = items.at(-1)
    if (last && !gotComfort) {
      set(
        put(
          list.value,
          message('today', last.text, last.ts, {
            ai: last.ai_generated,
            // 已经反馈过的不再问
            comfortId: last.feedback === null ? last.id : null,
            until: endOfDay(now()),
          }),
        ),
      )
    }
    if (rebuilt) notice(t('error.db_rebuilt'))
  }

  onBeforeUnmount(() => {
    clearTimeout(timer)
    for (const u of unlisten) u()
  })

  return { shown, error, notice, comfort, dismiss, init }
}
