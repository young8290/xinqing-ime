<script setup lang="ts">
// 日程的编辑框（06 FR-SCH-05“修改”、FR-SCH-09 新建与编辑）：标题、日期、开始、结束、全天、地点、截止、提醒。
// 先按后端同样的规则校验（agenda.ts），说清哪里不对；保存后时间重叠只提示、不阻止（FR-SCH-11 第 3 条）。
import { ref } from 'vue'
import type { ScheduleInput } from '@/api'
import { t } from '@/i18n'
import { REMIND_CHOICES, SCHEDULE_TITLE_MAX, scheduleInput, type ScheduleForm } from './agenda'

const props = defineProps<{ initial: ScheduleForm; busy?: boolean }>()
const emit = defineEmits<{ save: [input: ScheduleInput]; cancel: [] }>()

const f = ref<ScheduleForm>({ ...props.initial })
const error = ref<string | null>(null)

function submit(): void {
  const r = scheduleInput(f.value)
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
    <label class="wide">
      <span>{{ t('dashboard.schedule.title') }}</span>
      <input
        v-model="f.title"
        class="d-input"
        :placeholder="t('dashboard.schedule.title_hint', { n: SCHEDULE_TITLE_MAX })"
      />
    </label>
    <label>
      <span>{{ t('dashboard.schedule.date') }}</span>
      <input v-model="f.date" class="d-input" type="date" />
    </label>
    <label class="check">
      <input v-model="f.all_day" type="checkbox" />
      <span>{{ t('dashboard.schedule.all_day') }}</span>
    </label>
    <template v-if="!f.all_day">
      <label>
        <span>{{ t('dashboard.schedule.time') }}</span>
        <input v-model="f.time" class="d-input" type="time" />
      </label>
      <label v-if="!f.is_deadline">
        <span>{{ t('dashboard.schedule.end_time') }}</span>
        <input v-model="f.end_time" class="d-input" type="time" />
      </label>
      <label class="check">
        <input v-model="f.is_deadline" type="checkbox" />
        <span>{{ t('dashboard.schedule.deadline') }}</span>
      </label>
    </template>
    <label class="wide">
      <span>{{ t('dashboard.schedule.location') }}</span>
      <input v-model="f.location" class="d-input" />
    </label>
    <label v-if="!f.all_day">
      <span>{{ t('dashboard.schedule.remind') }}</span>
      <select v-model="f.remind" class="d-input">
        <option v-for="r in REMIND_CHOICES" :key="r" :value="r">
          {{ r === 'default' ? t('dashboard.schedule.remind_default') : t(`dashboard.schedule.remind_${r}`) }}
        </option>
      </select>
    </label>
    <p v-if="error" class="d-error wide" role="alert">{{ error }}</p>
    <div class="d-row wide">
      <button type="submit" class="primary" :disabled="props.busy">{{ t('dashboard.schedule.save') }}</button>
      <button type="button" @click="emit('cancel')">{{ t('dashboard.schedule.cancel') }}</button>
    </div>
  </form>
</template>

<style scoped>
.editor {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: var(--xq-sp-2) var(--xq-sp-3);
  align-items: end;
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

.check {
  flex-direction: row;
  align-items: center;
  min-height: 32px;
}

.wide {
  grid-column: 1 / -1;
}

.d-error {
  margin: 0;
}
</style>
