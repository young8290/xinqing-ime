<script setup lang="ts">
// 卡片层（07 第 2 节、FR-WGT-07，16 D-04）：依附小组件的透明窗口，最多同时 2 张卡片，更多的排队。
// 卡片类型：日程、待办、到点提醒、错过的提醒、休息提醒、自评回应、晚间小结、周信提示；专注结束（FR-RST-10，P2）随专注计时再加。
// 卡片出现时不抢当前应用的焦点；点进来以后 Enter 执行主操作、Esc 收起。位置与显示由 useCardsWindow.ts 管。
import { computed, onMounted, ref } from 'vue'
import { t } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'
import NoteCard from './NoteCard.vue'
import type { Card } from './queue'
import ReminderCard from './ReminderCard.vue'
import RestCard from './RestCard.vue'
import ScheduleCard from './ScheduleCard.vue'
import { useCards } from './useCards'
import { useCardsWindow } from './useCardsWindow'

const settings = useSettingsStore()
const cards = useCards()
const a = cards.actions
const stack = ref<HTMLElement | null>(null)
const topmost = computed(() => settings.values['widget.topmost'] !== false)
const place = useCardsWindow(cards.count, stack, topmost)

const err = (c: Card) => cards.errors.value[c.key] ?? null

/** 卡片上的两个按钮：主操作与次要操作（自评回应、晚间小结、周信） */
function notePrimary(c: Card): void {
  if (c.kind === 'self_reply') void a.chat(c.key)
  else if (c.kind === 'evening') void a.openToday(c.key)
  else if (c.kind === 'letter') void a.openLetter(c.key, c.letter.id)
}

function noteSecondary(c: Card): void {
  // 周信“等会儿再看”：留一条一句话区提示；其余直接收起
  if (c.kind === 'letter') void a.letterLater(c.key, c.letter.id)
  else a.dismiss(c.key)
}

onMounted(async () => {
  place.start()
  settings.init(['widget.topmost']).catch((e) => console.warn('读取置顶设置失败', e))
  try {
    await cards.init()
  } catch (e) {
    console.warn('订阅卡片事件失败', e)
  }
})
</script>

<template>
  <div
    ref="stack"
    class="stack"
    :class="{ below: place.below.value }"
    role="region"
    :aria-label="t('cards.region')"
    @pointerenter="cards.hovering.value = true"
    @pointerleave="cards.hovering.value = false"
  >
    <template v-for="c in cards.view.value.shown" :key="c.key">
      <ScheduleCard
        v-if="c.kind === 'schedule' || c.kind === 'todo'"
        :card="c"
        :error="err(c)"
        @add="
          c.item && (c.kind === 'schedule' ? a.scheduleAdd(c.key, c.item.id) : a.todoAdd(c.key, c.item.id))
        "
        @edit="
          c.item && (c.kind === 'schedule' ? a.scheduleEdit(c.key, c.item.id) : a.todoEdit(c.key, c.item.id))
        "
        @ignore="
          c.item &&
          (c.kind === 'schedule' ? a.scheduleIgnore(c.key, c.item.id) : a.todoIgnore(c.key, c.item.id))
        "
        @dismiss="a.dismiss(c.key)"
      />
      <ReminderCard
        v-else-if="c.kind === 'reminder'"
        :due="c.due"
        :error="err(c)"
        @act="(x) => a.reminder(c.key, c.due.id, x)"
        @dismiss="a.dismiss(c.key)"
      />
      <ReminderCard
        v-else-if="c.kind === 'missed'"
        :missed="c.missed"
        :error="err(c)"
        @open="a.openSchedule(c.key)"
        @dismiss="a.dismiss(c.key)"
      />
      <RestCard
        v-else-if="c.kind === 'rest'"
        :due="c.due"
        :error="err(c)"
        @act="(x) => a.rest(c.due.kind, x)"
      />
      <NoteCard
        v-else
        :card="c"
        :error="err(c)"
        @primary="notePrimary(c)"
        @secondary="noteSecondary(c)"
        @dismiss="noteSecondary(c)"
      />
    </template>
    <p v-if="cards.view.value.waiting > 0" class="more">
      {{ t('cards.more', { n: cards.view.value.waiting }) }}
    </p>
  </div>
</template>

<style>
/* 卡片层窗口是透明的，卡片之外不铺底色 */
html,
body {
  background: transparent;
  overflow: hidden;
}
</style>

<style scoped>
/* 在小组件上方时第一张卡片离小组件最近（排在最下面）；在下方时排在最上面 */
.stack {
  display: flex;
  flex-direction: column-reverse;
  gap: var(--xq-sp-2);
}

.stack.below {
  flex-direction: column;
}

.more {
  margin: 0;
  padding: 0 var(--xq-sp-3);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
  text-align: right;
}
</style>
