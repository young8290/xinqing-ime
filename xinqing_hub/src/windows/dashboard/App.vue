<script setup lang="ts">
// 情绪看板（07 FR-DSH-01～06，16 D-07）：左侧导航六页，右侧内容，布局与设置中心一致。
// 看板不展示用户在别的程序里打的字（FR-DSH-01，库里本来也没有）；日记与对话是用户在心晴里写的，按 FR-DIA-03 查看。
// 别的窗口要看板打开某一页（底栏、卡片层、一句话区）时经 shared/bus.ts 传 `{ page, edit?, letter? }`；
// 地址栏 #weekly 等也能直接打开对应页。
import { onBeforeUnmount, onMounted, ref, type Component } from 'vue'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { t, type CopyKey } from '@/i18n'
import CalendarPage from './CalendarPage.vue'
import ChatDiaryPage from './ChatDiaryPage.vue'
import MailboxPage from './MailboxPage.vue'
import SchedulePage from './SchedulePage.vue'
import TodayPage from './TodayPage.vue'
import WeeklyPage from './WeeklyPage.vue'
import { onDashboardNav, takeDashboardNav, type DashboardNav, type DashboardPage } from '../shared/bus'
import './dashboard.css'

const PAGES: { id: DashboardPage; title: CopyKey; component: Component }[] = [
  { id: 'today', title: 'dashboard.nav.today', component: TodayPage },
  { id: 'calendar', title: 'dashboard.nav.calendar', component: CalendarPage },
  { id: 'weekly', title: 'dashboard.nav.weekly', component: WeeklyPage },
  { id: 'schedule', title: 'dashboard.nav.schedule', component: SchedulePage },
  { id: 'mailbox', title: 'dashboard.nav.mailbox', component: MailboxPage },
  { id: 'chat_diary', title: 'dashboard.nav.chat_diary', component: ChatDiaryPage },
]

const fromHash = PAGES.find((p) => `#${p.id}` === location.hash)?.id
const initial = takeDashboardNav()
const current = ref<DashboardPage>(initial?.page ?? fromHash ?? 'today')
/** 跳页时带来的定位（打开哪条日程的编辑框、哪封信）；只给对应的页 */
const nav = ref<DashboardNav | null>(initial)
let unlisten: UnlistenFn | null = null

const page = () => PAGES.find((p) => p.id === current.value)!

function pageProps(): Record<string, unknown> {
  const n = nav.value
  if (!n || n.page !== current.value) return {}
  if (n.page === 'schedule' && n.edit) return { edit: n.edit }
  if (n.page === 'mailbox' && n.letter) return { letter: n.letter }
  return {}
}

function go(id: DashboardPage): void {
  current.value = id
  nav.value = null
}

onMounted(async () => {
  try {
    unlisten = await onDashboardNav((n) => {
      current.value = n.page
      nav.value = n
    })
  } catch (e) {
    console.warn('订阅看板跳页失败', e)
  }
})

onBeforeUnmount(() => unlisten?.())
</script>

<template>
  <div class="layout">
    <nav class="nav">
      <button
        v-for="p in PAGES"
        :key="p.id"
        class="nav-item"
        :class="{ active: current === p.id }"
        :aria-current="current === p.id ? 'page' : undefined"
        @click="go(p.id)"
      >
        {{ t(p.title) }}
      </button>
    </nav>
    <main class="page">
      <component :is="page().component" :key="current" v-bind="pageProps()" />
    </main>
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  height: 100%;
}

.nav {
  display: flex;
  flex: none;
  flex-direction: column;
  gap: var(--xq-sp-1);
  width: 180px;
  padding: var(--xq-sp-4) var(--xq-sp-3);
  border-right: 1px solid var(--xq-border);
  background: var(--xq-surface-2);
}

.nav-item {
  justify-content: flex-start;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-color: transparent;
  background: transparent;
  text-align: left;
}

.nav-item.active {
  background: var(--xq-surface);
}

.page {
  flex: 1;
  min-width: 0;
  padding: var(--xq-sp-5);
  overflow: auto;
}
</style>
