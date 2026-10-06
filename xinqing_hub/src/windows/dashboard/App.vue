<script setup lang="ts">
// 情绪看板（07 FR-DSH-01～06，16 D-07）：左侧导航六页，右侧内容，布局与设置中心一致。
// 已有：今日（当前天气、今天的自评）、周报（作息洞察）。情绪日历、日程与待办、信箱、对话与日记等对应命令到位后补。
// 看板中不展示任何用户输入的原文（FR-DSH-01）。地址栏 #weekly 等直接打开对应页。
import { ref, type Component } from 'vue'
import { t, type CopyKey } from '@/i18n'
import ComingSoon from './ComingSoon.vue'
import TodayPage from './TodayPage.vue'
import WeeklyPage from './WeeklyPage.vue'

type PageId = 'today' | 'calendar' | 'weekly' | 'schedule' | 'mailbox' | 'chat_diary'

const PAGES: { id: PageId; title: CopyKey; component: Component }[] = [
  { id: 'today', title: 'dashboard.today', component: TodayPage },
  { id: 'calendar', title: 'dashboard.calendar', component: ComingSoon },
  { id: 'weekly', title: 'dashboard.weekly', component: WeeklyPage },
  { id: 'schedule', title: 'dashboard.schedule', component: ComingSoon },
  { id: 'mailbox', title: 'dashboard.mailbox', component: ComingSoon },
  { id: 'chat_diary', title: 'dashboard.chat_diary', component: ComingSoon },
]

const fromHash = PAGES.find((p) => `#${p.id}` === location.hash)?.id
const current = ref<PageId>(fromHash ?? 'today')
const page = () => PAGES.find((p) => p.id === current.value)!
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
        @click="current = p.id"
      >
        {{ t(p.title) }}
      </button>
    </nav>
    <main class="page">
      <!-- 只有“准备中”页要页名；别的页不收 title，传了会落到根元素上变成悬停提示 -->
      <component
        :is="page().component"
        :key="current"
        v-bind="page().component === ComingSoon ? { title: page().title } : {}"
      />
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
