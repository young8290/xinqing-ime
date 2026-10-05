<script setup lang="ts">
// 快捷键录制框（07 FR-SET-02、ADR 0016 第 5 条）：点一下进入录制，按下组合键即保存；Esc 取消、单按退格清除。
// 只带 Shift 或不带修饰键的字符键会和打字冲突，不收；和别的快捷键重复时提示，不保存。
import { computed, ref } from 'vue'
import { t } from '@/i18n'
import { UNSET, formatHeld, formatHotkey, heldModifiers, normalizeHotkey, recordKey } from './hotkey'

const props = defineProps<{
  id: string
  /** 这一项的名称（读屏用） */
  name: string
  value: unknown
  /** 这个组合已经给了哪一项（返回那一项的名称），没有返回 null */
  takenBy: (combo: string) => string | null
}>()
const emit = defineEmits<{ save: [value: string] }>()

const recording = ref(false)
const held = ref('')
const problem = ref('')
/** 录制时处理过的 keydown 对应的 keyup 也要吞掉：否则松开空格会再“点”一次按钮，重新进入录制 */
let swallowKeyup = false

const current = computed(() => formatHotkey(props.value))
const isSet = computed(() => normalizeHotkey(props.value) !== null)
const shown = computed(() =>
  recording.value ? formatHeld(held.value) || t('ime.hotkey.recording') : current.value,
)

function start(): void {
  if (recording.value) return
  recording.value = true
  held.value = ''
  problem.value = ''
}

function stop(): void {
  recording.value = false
  held.value = ''
}

function onKeydown(e: KeyboardEvent): void {
  if (!recording.value) return
  e.preventDefault()
  e.stopPropagation()
  swallowKeyup = true
  const r = recordKey(e)
  switch (r.kind) {
    case 'pending':
      held.value = r.held
      return
    case 'cancel':
      stop()
      return
    case 'clear':
      stop()
      if (isSet.value) emit('save', UNSET)
      return
    case 'invalid':
      held.value = ''
      problem.value = t(`ime.hotkey.${r.reason}`)
      return
    case 'done': {
      stop()
      const owner = props.takenBy(r.value)
      if (owner !== null) {
        problem.value = t('ime.hotkey.taken', { name: owner })
        return
      }
      problem.value = ''
      if (r.value !== normalizeHotkey(props.value)) emit('save', r.value)
    }
  }
}

function onKeyup(e: KeyboardEvent): void {
  // 松开修饰键：更新“正在按着”的显示
  if (recording.value) held.value = heldModifiers(e).join('+')
  if (swallowKeyup) {
    e.preventDefault()
    swallowKeyup = false
  }
}

function clear(): void {
  problem.value = ''
  emit('save', UNSET)
}
</script>

<template>
  <div class="hotkey">
    <button
      :id="id"
      type="button"
      class="record"
      :class="{ recording }"
      :aria-pressed="recording"
      :aria-label="recording ? t('ime.hotkey.recording') : t('ime.hotkey.change', { name, value: current })"
      @click="start"
      @keydown="onKeydown"
      @keyup="onKeyup"
      @blur="stop"
    >
      <kbd>{{ shown }}</kbd>
    </button>
    <button v-if="isSet && !recording" type="button" class="clear compact" @click="clear">
      {{ t('ime.hotkey.clear') }}
    </button>
    <p v-if="problem" class="problem" role="alert">{{ problem }}</p>
  </div>
</template>

<style scoped>
.hotkey {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
}

.record {
  min-width: 160px;
  min-height: 32px;
  padding: 0 var(--xq-sp-3);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  color: var(--xq-text-1);
  text-align: left;
}

.record.recording {
  border-color: var(--xq-primary);
  background: var(--xq-surface-2);
}

kbd {
  font: inherit;
}

.recording kbd {
  color: var(--xq-text-2);
}

.compact {
  padding: 0 var(--xq-sp-2);
}

.problem {
  flex-basis: 100%;
  margin: 0;
  color: var(--xq-danger);
  font-size: var(--xq-fs-sm);
}
</style>
