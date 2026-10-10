<script setup lang="ts">
// 卡片的外框（07 FR-WGT-07）：标题、正文、按钮一行；键盘 Enter 执行主操作、Esc 收起。
// 出现时不抢焦点（卡片层窗口本身不聚焦）：用户点进来或 Tab 进来之后键盘才生效。
import { t } from '@/i18n'

const props = defineProps<{ label: string; error?: string | null; tag?: string | null }>()
const emit = defineEmits<{ primary: []; dismiss: [] }>()

function onEnter(e: KeyboardEvent): void {
  // 焦点在按钮或输入框上时按它自己的行为
  const el = e.target as HTMLElement | null
  if (el && ['BUTTON', 'INPUT', 'TEXTAREA', 'A'].includes(el.tagName)) return
  e.preventDefault()
  emit('primary')
}
</script>

<template>
  <section
    class="card"
    role="alertdialog"
    :aria-label="props.label"
    tabindex="-1"
    @keydown.esc.stop="emit('dismiss')"
    @keydown.enter="onEnter"
  >
    <header class="head">
      <div class="title"><slot name="title" /></div>
      <span v-if="props.tag" class="tag">{{ props.tag }}</span>
      <button
        class="close"
        :aria-label="t('common.close')"
        :title="t('common.close')"
        @click="emit('dismiss')"
      >
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      </button>
    </header>
    <div class="body"><slot /></div>
    <p v-if="props.error" class="error" role="alert">{{ props.error }}</p>
    <div v-if="$slots.actions" class="actions"><slot name="actions" /></div>
  </section>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-1);
  max-height: 220px; /* 07 第 2 节：卡片层 320 × ≤ 220 */
  padding: var(--xq-sp-3);
  overflow: hidden;
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
  background: var(--xq-surface);
}

.card:focus {
  outline: none;
}

.head {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
}

.title {
  flex: 1;
  min-width: 0;
  font-size: var(--xq-fs-lg);
  font-weight: 500;
  line-height: var(--xq-lh-lg);
}

/* DS-COPY-05：`AI 识别` / `AI 生成` */
.tag {
  flex: none;
  padding: 0 var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

/* 仍守 32 × 32 点击目标，用负边距不撑高标题行 */
.close {
  flex: none;
  margin: calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) 0;
  padding: 0;
  border-color: transparent;
  background: transparent;
  color: var(--xq-text-2);
}

.close path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
}

.body {
  min-height: 0;
  overflow: hidden;
  color: var(--xq-text-1);
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.body :deep(p) {
  margin: 0;
}

.body :deep(.muted) {
  color: var(--xq-text-2);
}

.error {
  margin: 0;
  color: var(--xq-danger);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-1);
  margin-top: var(--xq-sp-1);
}

/* 按钮仍守 32 × 32 的点击目标（DS-A11Y-03），只收窄左右留白 */
.actions :deep(button) {
  padding: 0 var(--xq-sp-3);
  font-size: var(--xq-fs-sm);
  white-space: nowrap;
}
</style>
