<script setup lang="ts">
// 桌面小组件（07 FR-WGT-01～06）。骨架阶段：状态行、小精灵、一句话区、离线角标、单击打开对话、拖动。
// 右键菜单、卡片层、底栏数据、贴边隐藏随 D-02 / D-04 补上。
import { computed, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { commands, unwrap } from '@/api'
import WeatherSprite from '@/components/WeatherSprite.vue'
import { errorText, t } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'
import { useStatusStore } from '@/stores/status'
import { statusLine } from './statusLine'

const status = useStatusStore()
const settings = useSettingsStore()
const line = computed(() => (status.snapshot ? statusLine(status.snapshot) : null))
const opacity = computed(() => {
  const v = settings.values['widget.opacity']
  return typeof v === 'number' ? v : 1
})
const message = ref(t('greeting.idle'))

onMounted(async () => {
  try {
    await Promise.all([status.init(), settings.init(['widget.opacity'])])
  } catch (e) {
    message.value = errorText(e)
  }
})

async function openChat(): Promise<void> {
  try {
    await unwrap(commands.openWindow('chat'))
  } catch (e) {
    message.value = errorText(e)
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
  <main
    class="widget"
    :style="{ opacity }"
    tabindex="0"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @keydown.enter="openChat"
  >
    <WeatherSprite v-if="line" :weather="line.weather" :eyes-closed="line.eyesClosed" />
    <div class="content">
      <p class="status" aria-live="polite" :title="line?.hint ?? undefined">{{ line?.text }}</p>
      <p class="message">{{ message }}</p>
      <footer class="footer" />
    </div>
    <span v-if="status.snapshot?.offline" class="badge">{{ t('widget.offline_badge') }}</span>
  </main>
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

.content {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
  height: 100%;
}

.status {
  margin: 0 0 var(--xq-sp-2);
  padding-right: var(--xq-sp-6);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
  position: absolute;
  top: var(--xq-sp-3);
  right: var(--xq-sp-3);
  padding: 0 var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
