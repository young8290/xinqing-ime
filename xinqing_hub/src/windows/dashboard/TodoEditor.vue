<script setup lang="ts">
// 待办的编辑框（06 FR-SCH-13）：标题（≤ 16 字）与可选的截止日期。
import { ref } from 'vue'
import type { TodoInput } from '@/api'
import { t } from '@/i18n'
import { TODO_TITLE_MAX, todoInput, type TodoForm } from './agenda'

const props = defineProps<{ initial: TodoForm; busy?: boolean }>()
const emit = defineEmits<{ save: [input: TodoInput]; cancel: [] }>()

const f = ref<TodoForm>({ ...props.initial })
const error = ref<string | null>(null)

function submit(): void {
  const r = todoInput(f.value)
  if (!r.ok) {
    error.value = t(r.error, r.vars)
    return
  }
  error.value = null
  emit('save', r.input)
}
</script>

<template>
  <form class="editor" @submit.prevent="submit" @keydown.esc.stop="emit('cancel')">
    <label class="grow">
      <span>{{ t('dashboard.schedule.title') }}</span>
      <input
        v-model="f.title"
        class="d-input"
        :placeholder="t('dashboard.schedule.title_hint', { n: TODO_TITLE_MAX })"
      />
    </label>
    <label>
      <span>{{ t('dashboard.schedule.due_date') }}</span>
      <input v-model="f.due_date" class="d-input" type="date" />
    </label>
    <div class="d-row">
      <button type="submit" class="primary" :disabled="props.busy">{{ t('dashboard.schedule.save') }}</button>
      <button type="button" @click="emit('cancel')">{{ t('dashboard.schedule.cancel') }}</button>
    </div>
    <p v-if="error" class="d-error full" role="alert">{{ error }}</p>
  </form>
</template>

<style scoped>
.editor {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2) var(--xq-sp-3);
  align-items: flex-end;
  padding: var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
}

label {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-1);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

label .d-input {
  background: var(--xq-surface);
}

.grow {
  flex: 1;
  min-width: 200px;
}

.full {
  flex-basis: 100%;
  margin: 0;
}
</style>
