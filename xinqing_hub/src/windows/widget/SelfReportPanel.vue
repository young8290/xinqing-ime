<script setup lang="ts">
// “我现在…”自评面板（04 FR-STA-10）：六个选项，点一下就提交；备注选填（≤ 50 字，只存本地），不追问原因。
// 盖住小组件的整个内容区（约 286 × 134 px）：标题、备注一行、两行选项刚好放下。
import { onMounted, ref } from 'vue'
import type { SelfWeather } from '@/api'
import { t } from '@/i18n'
import { NOTE_MAX, SELF_WEATHERS, selfOption } from '@/selfReport'

const emit = defineEmits<{ submit: [weather: SelfWeather, note: string]; close: [] }>()

const note = ref('')
const root = ref<HTMLElement | null>(null)

// 打开时把焦点放在第一个选项上：键盘用户直接 Tab 选择，Esc 关闭（DS-A11Y-01）
onMounted(() => root.value?.querySelector<HTMLButtonElement>('.option')?.focus())
</script>

<template>
  <div
    ref="root"
    class="self-report"
    role="dialog"
    :aria-label="t('widget.self_report_prompt')"
    @pointerdown.stop
    @keydown.esc.stop="emit('close')"
    @keydown.enter.stop
  >
    <div class="head">
      <p class="title">{{ t('widget.self_report_prompt') }}</p>
      <button class="compact close" :aria-label="t('common.close')" @click.stop="emit('close')">
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      </button>
    </div>
    <input
      v-model="note"
      class="note"
      type="text"
      :maxlength="NOTE_MAX"
      :placeholder="t('widget.self_report_note')"
      :aria-label="t('widget.self_report_note')"
    />
    <div class="options">
      <button
        v-for="w in SELF_WEATHERS"
        :key="w"
        class="option"
        :data-weather="w"
        @click.stop="emit('submit', w, note)"
      >
        {{ selfOption(w) }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.self-report {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-1);
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.title {
  margin: 0;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
  font-weight: 500;
}

.close {
  margin: calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) calc(-1 * var(--xq-sp-2)) 0;
  border-color: transparent;
  background: transparent;
}

.close path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
}

.note {
  min-height: 32px;
  padding: 0 var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-1);
  font: inherit;
  font-size: var(--xq-fs-xs);
}

.note::placeholder {
  color: var(--xq-text-3);
}

.options {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: var(--xq-sp-1);
}

/* 仍守 32 × 32 的点击目标（DS-A11Y-03） */
.option {
  min-width: 0;
  padding: 0 var(--xq-sp-1);
  font-size: var(--xq-fs-sm);
  white-space: nowrap;
}

.compact {
  padding: 0 var(--xq-sp-2);
}
</style>
