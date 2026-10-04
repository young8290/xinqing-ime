<script setup lang="ts">
// 桌面小组件（07 FR-WGT-01～06）：状态行、小精灵、一句话区、离线角标、单击打开对话、拖动吸附、
// 右键菜单、贴边隐藏、悬停状态行显示解释、“我现在…”自评。卡片层、底栏数据、一句话区的消息优先级随 D-02 / D-04 后续 PR 补上
// （进度见 docs/xinqing/handover/D-前端与视觉.md）。
import { computed, nextTick, onMounted, ref, toRef } from 'vue'
import { LogicalPosition, getCurrentWindow } from '@tauri-apps/api/window'
import { Menu } from '@tauri-apps/api/menu'
import { commands, unwrap, type SelfWeather, type Verdict } from '@/api'
import ExplainPanel from '@/components/ExplainPanel.vue'
import WeatherSprite from '@/components/WeatherSprite.vue'
import WeatherStage from '@/components/WeatherStage.vue'
import { errorText, t } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'
import { useStatusStore } from '@/stores/status'
import { menuEntries, type MenuAction } from './menu'
import SelfReportPanel from './SelfReportPanel.vue'
import { statusLine } from './statusLine'
import { useExplain } from './useExplain'
import { useSelfReport } from './useSelfReport'
import { useWidgetWindow } from './useWidgetWindow'

const status = useStatusStore()
const settings = useSettingsStore()
const selfReport = useSelfReport()
const line = computed(() => (status.snapshot ? statusLine(status.snapshot, selfReport.active.value) : null))
const setting = <T,>(key: string, fallback: T) =>
  computed(() => {
    const v = settings.values[key]
    return typeof v === typeof fallback ? (v as T) : fallback
  })
const opacity = setting('widget.opacity', 1)
const autohide = setting('widget.autohide', false)
const topmost = setting('widget.topmost', true)
const message = ref(t('greeting.idle'))
const menuOpen = ref(false)

// 菜单或自评面板开着时不贴边收起
const holdOpen = computed(() => menuOpen.value || selfReport.panelOpen.value)
const place = useWidgetWindow({ autohide, topmost, holdOpen })
const explain = useExplain(
  toRef(status, 'snapshot'),
  computed(() => selfReport.active.value !== null),
)
const penEl = ref<HTMLElement | null>(null)

/** 打开“我现在…”（状态行的 ✎ 或右键菜单，FR-WGT-06） */
function openSelfReport(): void {
  explain.close()
  selfReport.panelOpen.value = true
}

/** 关面板后把焦点还给 ✎，键盘用户不至于掉到窗口外 */
async function closeSelfReport(): Promise<void> {
  selfReport.panelOpen.value = false
  await nextTick()
  penEl.value?.focus()
}

async function onSelfSubmit(weather: SelfWeather, note: string): Promise<void> {
  try {
    await selfReport.submit(weather, note)
    await nextTick()
    penEl.value?.focus()
  } catch (e) {
    message.value = errorText(e)
  }
}
const panel = ref<{ $el: HTMLElement } | null>(null)

const statusEl = ref<HTMLElement | null>(null)
const thanksEl = ref<HTMLElement | null>(null)
// 投票后按钮换成致谢：被删掉的按钮如果有焦点，会发一个 relatedTarget 为空的 focusout，这期间不算离开
let swapping = false

/** 焦点在状态行与面板里的按钮之间移动不算离开；移出这两处才收起 */
function onFocusOut(e: FocusEvent): void {
  if (swapping) return
  const to = e.relatedTarget
  if (to instanceof Node && (statusEl.value?.contains(to) || panel.value?.$el.contains(to))) return
  explain.close()
}

/** “准 / 不准”（FR-STA-07）：记到最近一条状态记录（即当前显示的状态），“不准”会让后端上调个人阈值 */
async function vote(verdict: Verdict): Promise<void> {
  try {
    await unwrap(commands.submitFeedback('mood_state', null, verdict))
    const hadFocus = panel.value?.$el.contains(document.activeElement) ?? false
    swapping = true
    explain.voted.value = true
    await nextTick()
    swapping = false
    // 焦点原来在按钮上（键盘用户）：交给致谢那句，Esc 照样能收起面板
    if (hadFocus) thanksEl.value?.focus()
  } catch (e) {
    message.value = errorText(e)
  }
}

onMounted(async () => {
  void place.start()
  try {
    await Promise.all([status.init(), settings.init(['widget.opacity', 'widget.autohide', 'widget.topmost'])])
  } catch (e) {
    message.value = errorText(e)
  }
  // 自评覆盖期取不到时只是少显示“你说的”，不打扰用户
  selfReport.init().catch((e) => console.warn('读取自评失败', e))
})

async function run(action: () => Promise<unknown>): Promise<void> {
  try {
    await action()
  } catch (e) {
    message.value = errorText(e)
  }
}

const openChat = () => run(() => unwrap(commands.openWindow('chat')))

const ACTIONS: Record<MenuAction, () => Promise<unknown>> = {
  self_report: async () => openSelfReport(),
  pause: () => status.setPaused(true),
  resume: () => status.setPaused(false),
  dashboard: () => unwrap(commands.openWindow('dashboard')),
  settings: () => unwrap(commands.openWindow('settings')),
  hide: () => getCurrentWindow().hide(),
}

/** 系统原生菜单：能伸出小组件窗口之外，读屏和高对比度也由系统负责。`at` 省略时在鼠标处弹出。 */
async function openMenu(at?: LogicalPosition): Promise<void> {
  if (menuOpen.value) return
  menuOpen.value = true
  try {
    const items = menuEntries(status.snapshot).map(({ id, text }) => ({
      id,
      text,
      action: () => void run(ACTIONS[id]),
    }))
    const menu = await Menu.new({ items })
    await menu.popup(at)
  } catch (e) {
    message.value = errorText(e)
  } finally {
    menuOpen.value = false
  }
}

function onKeydown(e: KeyboardEvent): void {
  // FR-WGT-06：Shift+F10（以及键盘上的菜单键）在小组件左上角打开右键菜单
  if ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu') {
    e.preventDefault()
    void openMenu(new LogicalPosition(16, 16))
  } else if (e.key === 'Enter') {
    void openChat()
  } else if (e.key === 'Escape') {
    explain.close()
  }
}

// 按下后移动超过 4 px 视为拖动窗口，否则松开时算单击（FR-WGT-06：左键单击打开对话）
const DRAG_THRESHOLD = 4
let press: { x: number; y: number } | null = null

function onPointerDown(e: PointerEvent): void {
  if (e.button === 0) press = { x: e.screenX, y: e.screenY }
}

function onPointerMove(e: PointerEvent): void {
  if (!press) return
  if (Math.hypot(e.screenX - press.x, e.screenY - press.y) > DRAG_THRESHOLD) {
    press = null
    void getCurrentWindow().startDragging()
  }
}

function onPointerUp(): void {
  if (press) void openChat()
  press = null
}
</script>

<template>
  <div class="root" @pointerenter="place.pointerEnter" @pointerleave="place.pointerLeave">
    <button
      v-if="place.collapsed.value"
      class="tab"
      :style="{ opacity }"
      :aria-label="t('widget.expand')"
      @click="place.expand"
      @focus="place.expand"
    >
      <WeatherSprite v-if="line" :weather="line.weather" :eyes-closed="line.eyesClosed" :size="20" />
    </button>
    <main
      v-else
      class="widget"
      :style="{ opacity }"
      tabindex="0"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @contextmenu.prevent="openMenu()"
      @keydown="onKeydown"
    >
      <WeatherStage v-if="line" :weather="line.weather" :eyes-closed="line.eyesClosed" />
      <div class="content">
        <!-- 状态行：悬停或键盘聚焦时显示解释（FR-WGT-06、FR-STA-09）；右侧依次是离线角标和 ✎“我现在…” -->
        <div class="status-row">
          <p
            ref="statusEl"
            class="status"
            :title="line?.text"
            tabindex="0"
            aria-live="polite"
            :aria-describedby="explain.lines.value ? 'xq-explain' : undefined"
            @pointerenter="explain.hover"
            @pointerleave="explain.leave"
            @focus="explain.open"
            @focusout="onFocusOut"
          >
            {{ line?.text }}
          </p>
          <span v-if="status.snapshot?.offline" class="badge">{{ t('widget.offline_badge') }}</span>
          <button
            ref="penEl"
            class="pen"
            :aria-label="t('widget.self_report_prompt')"
            :title="t('widget.self_report_prompt')"
            @pointerdown.stop
            @keydown.enter.stop
            @click.stop="openSelfReport"
          >
            <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
              <path d="M3 13l.8-3.2L10.6 3a1.4 1.4 0 0 1 2 2L5.8 11.8z" />
            </svg>
          </button>
        </div>
        <SelfReportPanel
          v-if="selfReport.panelOpen.value"
          class="overlay"
          @submit="onSelfSubmit"
          @close="closeSelfReport"
        />
        <ExplainPanel
          v-if="explain.lines.value"
          id="xq-explain"
          ref="panel"
          class="overlay"
          role="tooltip"
          :lines="explain.lines.value"
          @pointerenter="explain.hover"
          @pointerleave="explain.leave"
          @focusout="onFocusOut"
        >
          <template #actions>
            <span v-if="explain.voted.value" ref="thanksEl" class="voted" role="status" tabindex="-1">{{
              t('widget.feedback.thanks')
            }}</span>
            <!-- 按钮上按下不算“单击小组件打开对话”，也不开始拖动 -->
            <div v-else class="vote" role="group" :aria-label="t('widget.feedback.label')" @pointerdown.stop>
              <button class="compact" @click.stop="vote('fit')" @keydown.enter.stop>
                {{ t('widget.feedback.fit') }}
              </button>
              <button class="compact" @click.stop="vote('unfit')" @keydown.enter.stop>
                {{ t('widget.feedback.unfit') }}
              </button>
            </div>
          </template>
        </ExplainPanel>
        <!-- FR-WGT-04：最多 2 行，悬停显示全文 -->
        <p class="message" :title="message">{{ message }}</p>
        <footer class="footer" />
      </div>
    </main>
  </div>
</template>

<style>
/* 小组件窗口是透明的，圆角卡片之外不铺底色 */
html,
body {
  background: transparent;
  overflow: hidden;
}
</style>

<style scoped>
.root {
  height: 100%;
}

.widget {
  position: relative;
  display: flex;
  gap: var(--xq-sp-3);
  align-items: center;
  height: 100%;
  padding: var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-window);
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-card);
  user-select: none;
  cursor: default;
}

/* 贴边隐藏后的小标签：24 px 宽，只露出小精灵（FR-WGT-01）。窗口本身就是标签大小 */
.tab {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  min-width: 0;
  height: 100%;
  padding: 0;
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
}

.content {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  height: 100%;
}

.status-row {
  display: flex;
  gap: var(--xq-sp-1);
  align-items: center;
  margin: 0 0 var(--xq-sp-2);
}

.status {
  flex: 1;
  min-width: 0;
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ✎ 仍守 32 × 32 的点击目标（DS-A11Y-03），用负边距不撑高 20 px 的状态行 */
.pen {
  flex: none;
  margin: calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) 0;
  padding: 0;
  border-color: transparent;
  background: transparent;
  color: var(--xq-text-2);
}

.pen path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linejoin: round;
}

/*
 * 解释面板与自评面板都盖住卡片的整个内容区（约 286 × 134 px）：解释最多 3 条说明、两行附注再加 32 px 高的
 * “准 / 不准”，自评是标题、备注和两行选项，只有这么宽才放得下（只盖右侧那一列时高度不够）。
 */
.overlay {
  position: absolute;
  inset: var(--xq-sp-4);
  z-index: 1;
  background: var(--xq-surface);
}

.vote {
  display: flex;
  flex: none;
  gap: var(--xq-sp-1);
}

/* 仍守 32 × 32 的点击目标（DS-A11Y-03），只收窄左右留白 */
.compact {
  padding: 0 var(--xq-sp-2);
  font-size: var(--xq-fs-xs);
}

.voted {
  flex: none;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.message {
  display: -webkit-box;
  flex: 1;
  margin: 0;
  overflow: hidden;
  font-size: var(--xq-fs-lg);
  font-weight: 500;
  line-height: var(--xq-lh-lg);
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2; /* FR-WGT-04：最多 2 行 */
}

.footer {
  min-height: var(--xq-lh-xs);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.badge {
  flex: none;
  padding: 0 var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
