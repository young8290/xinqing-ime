// 小组件底栏（07 FR-WGT-05）的取数：今日输入时长（day_stats）、已添加的日程、未完成待办数。
// 每分钟刷新一次；到点提醒、新的待办与日程确认后、窗口重新获得焦点时也刷新。取不到就不显示，不打扰用户。
import { onBeforeUnmount, ref } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { commands, events, unwrap } from '@/api'
import { localDate } from '@/format'
import { footerLeft, footerRight, nextSchedule } from './footer'

export const REFRESH_MS = 60_000

export function useFooter(now: () => number = Date.now) {
  const left = ref<string | null>(null)
  const right = ref<string | null>(null)
  let timer: ReturnType<typeof setInterval> | undefined
  const unlisten: UnlistenFn[] = []

  async function refresh(): Promise<void> {
    try {
      const [stats, schedules, todos] = await Promise.all([
        unwrap(commands.dayStats(localDate(new Date(now())))),
        unwrap(commands.scheduleList('added')),
        unwrap(commands.todoList('open')),
      ])
      left.value = footerLeft(stats.typing_min)
      right.value = footerRight(nextSchedule(schedules, now()), todos.length, now())
    } catch (e) {
      console.warn('读取底栏数据失败', e)
    }
  }

  async function init(): Promise<void> {
    timer = setInterval(() => void refresh(), REFRESH_MS)
    const again = () => void refresh()
    window.addEventListener('focus', again)
    unlisten.push(() => window.removeEventListener('focus', again))
    unlisten.push(await events.reminderDue.listen(again), await events.todoReady.listen(again))
    await refresh()
  }

  onBeforeUnmount(() => {
    clearInterval(timer)
    for (const u of unlisten) u()
  })

  return { left, right, refresh, init }
}
