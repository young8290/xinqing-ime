<script setup lang="ts">
// 输入法配置的一项（07 FR-SET-02 的类型 → 控件表）：布尔开关、枚举下拉、数字输入、文本、字符串列表；
// map / array / 认不出的类型只读显示 JSON。数值没有范围信息，所以不给滑块（ADR 0016 第 3 条）。
// 改动即提交（FR-SET-01 修改即保存），父组件负责调 ime_config_set；核心跳过时 `error` 显示在控件下方。
import { computed, ref } from 'vue'
import type { ImeField } from '@/api'
import { t } from '@/i18n'
import { fieldLabel, optionLabel, parseNumber } from './imeForm'

const props = defineProps<{ field: ImeField; value: unknown; error?: string | null }>()
const emit = defineEmits<{ save: [value: unknown] }>()

const id = computed(() => `ime-${props.field.key.replaceAll('.', '-')}`)
const label = computed(() => fieldLabel(props.field.key))
const list = computed(() =>
  Array.isArray(props.value) ? props.value.filter((v): v is string => typeof v === 'string') : [],
)
const json = computed(() => JSON.stringify(props.value ?? null, null, 2))
const draft = ref('')

function onNumber(e: Event): void {
  const kind = props.field.kind === 'int' ? 'int' : 'float'
  const n = parseNumber((e.target as HTMLInputElement).value, kind)
  if (n !== null && n !== props.value) emit('save', n)
}

function onText(e: Event): void {
  const v = (e.target as HTMLInputElement).value
  if (v !== props.value) emit('save', v)
}

function addItem(): void {
  const v = draft.value.trim()
  if (v === '' || list.value.includes(v)) return
  emit('save', [...list.value, v])
  draft.value = ''
}

function removeItem(item: string): void {
  emit(
    'save',
    list.value.filter((v) => v !== item),
  )
}
</script>

<template>
  <div class="row" :data-key="field.key">
    <label class="label" :for="id">{{ label }}</label>
    <div class="control">
      <input
        v-if="field.kind === 'bool'"
        :id="id"
        type="checkbox"
        role="switch"
        :checked="value === true"
        @change="emit('save', ($event.target as HTMLInputElement).checked)"
      />
      <select
        v-else-if="field.kind === 'enum'"
        :id="id"
        :value="value"
        @change="emit('save', ($event.target as HTMLSelectElement).value)"
      >
        <option v-for="o in field.options ?? []" :key="o" :value="o">{{ optionLabel(field.key, o) }}</option>
      </select>
      <input
        v-else-if="field.kind === 'int' || field.kind === 'float'"
        :id="id"
        type="number"
        :step="field.kind === 'int' ? 1 : 0.5"
        :value="value"
        @change="onNumber"
      />
      <input v-else-if="field.kind === 'string'" :id="id" type="text" :value="value ?? ''" @change="onText" />
      <div v-else-if="field.kind === 'string_list'" class="list">
        <span v-for="item in list" :key="item" class="chip">
          {{ item }}
          <button class="remove" :aria-label="t('ime.list_remove', { item })" @click="removeItem(item)">
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
              <path d="M3 3l6 6M9 3l-6 6" />
            </svg>
          </button>
        </span>
        <input :id="id" v-model="draft" type="text" class="add" @keydown.enter.prevent="addItem" />
        <button class="compact" @click="addItem">{{ t('ime.list_add') }}</button>
      </div>
      <template v-else>
        <pre :id="id" class="json" tabindex="0">{{ json }}</pre>
        <p class="hint">{{ t('ime.readonly') }}</p>
      </template>
      <p v-if="error" class="error" role="alert">{{ error }}</p>
    </div>
  </div>
</template>

<style scoped>
.row {
  display: grid;
  grid-template-columns: minmax(120px, 200px) 1fr;
  gap: var(--xq-sp-3);
  align-items: start;
  padding: var(--xq-sp-2) 0;
  border-bottom: 1px solid var(--xq-border);
}

.label {
  min-height: 32px;
  padding-top: var(--xq-sp-1);
  overflow-wrap: anywhere;
}

.control {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-1);
  min-width: 0;
}

.control > input[type='checkbox'] {
  width: 20px;
  height: 20px;
  margin: 6px 0;
}

select,
input[type='number'],
input[type='text'] {
  min-height: 32px;
  max-width: 320px;
  padding: 0 var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  color: var(--xq-text-1);
  font: inherit;
}

.list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-1);
  align-items: center;
}

.chip {
  display: inline-flex;
  gap: var(--xq-sp-1);
  align-items: center;
  padding-left: var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
}

.remove {
  min-width: 28px;
  min-height: 28px;
  padding: 0;
  border-color: transparent;
  background: transparent;
}

.remove path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
}

.add {
  width: 120px;
}

.compact {
  padding: 0 var(--xq-sp-2);
}

.json {
  max-height: 160px;
  margin: 0;
  padding: var(--xq-sp-2);
  overflow: auto;
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.hint {
  margin: 0;
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
}

.error {
  margin: 0;
  color: var(--xq-danger);
  font-size: var(--xq-fs-sm);
}
</style>
