<script setup lang="ts">
// 设置中心 · 关怀（07 FR-SET-05）：主动关怀频率（FR-CMF-06）、说话风格（FR-CHT-10）、状态摘要发给大模型（同意 ④，可撤回）、
// 安静时段与勿扰应用（FR-CMF-01 第 5 条，ADR 0019）、学校心理中心电话（FR-SAF-03）、晴晴记住的事（FR-CHT-07 第 3 条）。
// 改了就存。列表类设置每次提交整份列表；后端对不合格的值只回笼统的错误，所以先用 care.ts 按同样的规则拦一遍，说清哪里不对。
// 晚间小结、周信的开关要等设置键注册表登记（C），登记后补在“说话风格”下面。
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { commands, unwrap, type MemoryItem, type SettingValue } from '@/api'
import { errorText, t } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'
import {
  APP_MAX_CHARS,
  MEMORY_ENTRY_CHARS,
  MEMORY_MAX,
  PHONE_MAX_CHARS,
  crossesMidnight,
  hasApp,
  quietRange,
  showRange,
  validApp,
  validMemory,
  validPhone,
} from './care'

const LEVELS = ['more', 'normal', 'less', 'off'] as const
const STYLES = ['gentle', 'lively', 'brief'] as const
const KEYS = ['care.level', 'care.style', 'care.quiet_hours', 'care.dnd_apps', 'safety.school_phone']
type Field = 'quiet' | 'dnd' | 'phone' | 'memory'

const settings = useSettingsStore()
const error = ref<string | null>(null)
/** 各块自己的提示，显示在出错的那一块下面 */
const fieldError = reactive<Record<Field, string | null>>({
  quiet: null,
  dnd: null,
  phone: null,
  memory: null,
})
/** 列表最多几项，取自 settings_schema；读不到时按注册表现在的值 */
const maxItems = reactive({ 'care.quiet_hours': 4, 'care.dnd_apps': 32 })

const text = (key: string, fallback: string) => {
  const v = settings.values[key]
  return typeof v === 'string' ? v : fallback
}
const list = (key: string) => {
  const v = settings.values[key]
  return Array.isArray(v) ? v : []
}
const level = computed(() => text('care.level', 'normal'))
const style = computed(() => text('care.style', 'gentle'))
const quiet = computed(() => list('care.quiet_hours'))
const dnd = computed(() => list('care.dnd_apps'))
const phone = computed(() => text('safety.school_phone', ''))

// 同意 ④：null 表示还没读到，复选框先不可点
const summary = ref<boolean | null>(null)
const withdrawn = ref(false)

const memories = ref<MemoryItem[]>([])
const editing = ref<number | null>(null)
const draft = ref('')
const confirming = ref<number | null>(null)
const editInput = ref<HTMLInputElement[]>([])

const qFrom = ref('22:00')
const qTo = ref('07:00')
const newApp = ref('')

onMounted(async () => {
  window.addEventListener('focus', loadMemories)
  try {
    await Promise.all([settings.init(KEYS), loadConsent(), loadMemories(), loadSchema()])
  } catch (e) {
    error.value = errorText(e)
  }
})
onBeforeUnmount(() => window.removeEventListener('focus', loadMemories))

async function loadSchema(): Promise<void> {
  for (const f of await commands.settingsSchema()) {
    if (f.kind.type === 'list' && f.key in maxItems)
      maxItems[f.key as keyof typeof maxItems] = f.kind.max_items
  }
}

async function loadConsent(): Promise<void> {
  const state = await unwrap(commands.consentGet())
  summary.value = state.items.find((i) => i.item === 'llm_summary')?.granted ?? false
}

/** 聊天窗口里可能刚记下新的事，回到设置窗口时重读一次 */
async function loadMemories(): Promise<void> {
  try {
    memories.value = await unwrap(commands.memoryList())
  } catch (e) {
    fieldError.memory = errorText(e)
  }
}

async function save(key: string, value: SettingValue, field?: Field): Promise<boolean> {
  error.value = null
  if (field) fieldError[field] = null
  try {
    await settings.set(key, value)
    return true
  } catch (e) {
    if (field) fieldError[field] = errorText(e)
    else error.value = errorText(e)
    return false
  }
}

async function setSummary(granted: boolean): Promise<void> {
  error.value = null
  try {
    const state = await unwrap(commands.consentSet('llm_summary', granted))
    summary.value = state.items.find((i) => i.item === 'llm_summary')?.granted ?? granted
    // 撤回时如实说明：已经发出去的收不回来（FR-DAT-05）
    withdrawn.value = !granted
  } catch (e) {
    error.value = errorText(e)
  }
}

async function addQuiet(): Promise<void> {
  const r = quietRange(qFrom.value, qTo.value)
  if (!r) {
    fieldError.quiet = t('settings.care.quiet_invalid')
    return
  }
  if (quiet.value.includes(r)) {
    fieldError.quiet = null
    return
  }
  await save('care.quiet_hours', [...quiet.value, r], 'quiet')
}

async function addApp(): Promise<void> {
  const a = newApp.value.trim()
  if (!validApp(a)) {
    fieldError.dnd = t('settings.care.dnd_invalid')
    return
  }
  if (hasApp(dnd.value, a)) {
    fieldError.dnd = t('settings.care.dnd_duplicate')
    return
  }
  if (await save('care.dnd_apps', [...dnd.value, a], 'dnd')) newApp.value = ''
}

async function savePhone(raw: string): Promise<void> {
  const p = raw.trim()
  if (!validPhone(p)) {
    fieldError.phone = t('settings.care.phone_invalid')
    return
  }
  if (p !== phone.value) await save('safety.school_phone', p, 'phone')
  else fieldError.phone = null
}

async function startEdit(m: MemoryItem): Promise<void> {
  editing.value = m.id
  draft.value = m.content
  confirming.value = null
  fieldError.memory = null
  await nextTick()
  editInput.value[0]?.focus()
}

async function saveEdit(m: MemoryItem): Promise<void> {
  if (!validMemory(draft.value)) {
    fieldError.memory = t('error.memory_invalid')
    return
  }
  const content = draft.value.trim()
  fieldError.memory = null
  try {
    await unwrap(commands.memoryUpdate(m.id, content))
    m.content = content
    editing.value = null
  } catch (e) {
    fieldError.memory = errorText(e)
  }
}

/** 删除不可撤销：第一次点只把按钮换成“确定删掉”，再点才删 */
async function remove(m: MemoryItem): Promise<void> {
  if (confirming.value !== m.id) {
    confirming.value = m.id
    return
  }
  fieldError.memory = null
  try {
    await unwrap(commands.memoryDelete(m.id))
    memories.value = memories.value.filter((x) => x.id !== m.id)
  } catch (e) {
    fieldError.memory = errorText(e)
  } finally {
    confirming.value = null
  }
}

const day = (ts: number) => new Date(ts).toLocaleDateString('zh-CN')
</script>

<template>
  <section>
    <h1>{{ t('settings.care_nav') }}</h1>
    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <fieldset class="group" data-group="level">
      <legend>{{ t('settings.care.level') }}</legend>
      <div class="choices">
        <label v-for="c in LEVELS" :key="c" class="choice">
          <input type="radio" name="care-level" :checked="level === c" @change="save('care.level', c)" />
          {{ t(`care_level.${c}`) }}
        </label>
      </div>
      <p class="hint">{{ t('settings.care.level_hint') }}</p>
    </fieldset>

    <fieldset class="group" data-group="style">
      <legend>{{ t('settings.care.style') }}</legend>
      <label v-for="s in STYLES" :key="s" class="choice style">
        <input type="radio" name="care-style" :checked="style === s" @change="save('care.style', s)" />
        <span>
          {{ t(`settings.care.style_${s}`) }}
          <span class="eg">{{ t(`settings.care.style_${s}_eg`) }}</span>
        </span>
      </label>
    </fieldset>

    <fieldset class="group" data-group="summary">
      <legend>{{ t('settings.care.summary') }}</legend>
      <label class="choice">
        <input
          type="checkbox"
          :checked="summary === true"
          :disabled="summary === null"
          @change="setSummary(($event.target as HTMLInputElement).checked)"
        />
        {{ t('consent.llm_summary') }}
      </label>
      <p class="hint">{{ t('settings.care.summary_hint') }}</p>
      <p v-if="withdrawn" class="note" role="status">{{ t('error.withdraw_notice') }}</p>
    </fieldset>

    <fieldset class="group" data-group="quiet">
      <legend>{{ t('settings.care.quiet') }}</legend>
      <p class="hint">{{ t('settings.care.quiet_hint') }}</p>
      <ul v-if="quiet.length" class="items">
        <li v-for="r in quiet" :key="r" class="item">
          <span class="name">
            {{ showRange(r)
            }}<span v-if="crossesMidnight(r)" class="muted">{{ t('settings.care.quiet_next_day') }}</span>
          </span>
          <button
            class="icon"
            :aria-label="t('settings.care.quiet_remove', { range: showRange(r) })"
            @click="
              save(
                'care.quiet_hours',
                quiet.filter((x) => x !== r),
                'quiet',
              )
            "
          >
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
              <path d="M3 3l6 6M9 3l-6 6" />
            </svg>
          </button>
        </li>
      </ul>
      <form v-if="quiet.length < maxItems['care.quiet_hours']" class="add" @submit.prevent="addQuiet">
        <label class="inline">
          {{ t('settings.care.quiet_from') }}
          <input v-model="qFrom" type="time" required />
        </label>
        <label class="inline">
          {{ t('settings.care.quiet_to') }}
          <input v-model="qTo" type="time" required />
        </label>
        <button type="submit" class="compact">{{ t('settings.care.quiet_add') }}</button>
      </form>
      <p v-if="fieldError.quiet" class="error" role="alert">{{ fieldError.quiet }}</p>
    </fieldset>

    <fieldset class="group" data-group="dnd">
      <legend>{{ t('settings.care.dnd') }}</legend>
      <p class="hint">{{ t('settings.care.dnd_hint') }}</p>
      <ul v-if="dnd.length" class="items">
        <li v-for="a in dnd" :key="a" class="item">
          <span class="name">{{ a }}</span>
          <button
            class="icon"
            :aria-label="t('settings.care.dnd_remove', { app: a })"
            @click="
              save(
                'care.dnd_apps',
                dnd.filter((x) => x !== a),
                'dnd',
              )
            "
          >
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
              <path d="M3 3l6 6M9 3l-6 6" />
            </svg>
          </button>
        </li>
      </ul>
      <form v-if="dnd.length < maxItems['care.dnd_apps']" class="add" @submit.prevent="addApp">
        <input
          v-model="newApp"
          type="text"
          spellcheck="false"
          autocomplete="off"
          :maxlength="APP_MAX_CHARS"
          :aria-label="t('settings.care.dnd_placeholder')"
          :placeholder="t('settings.care.dnd_placeholder')"
        />
        <button type="submit" class="compact">{{ t('settings.care.dnd_add') }}</button>
      </form>
      <p v-if="fieldError.dnd" class="error" role="alert">{{ fieldError.dnd }}</p>
    </fieldset>

    <fieldset class="group" data-group="phone">
      <legend>{{ t('settings.care.phone') }}</legend>
      <input
        type="tel"
        class="phone"
        autocomplete="off"
        :maxlength="PHONE_MAX_CHARS"
        :aria-label="t('settings.care.phone')"
        :value="phone"
        @change="savePhone(($event.target as HTMLInputElement).value)"
      />
      <p class="hint">{{ t('settings.care.phone_hint') }}</p>
      <p v-if="fieldError.phone" class="error" role="alert">{{ fieldError.phone }}</p>
    </fieldset>

    <fieldset class="group" data-group="memory">
      <legend>
        {{ t('settings.care.memory') }}
        <span class="count">{{ t('settings.care.memory_count', { n: memories.length }) }}</span>
      </legend>
      <p class="hint">{{ t('settings.care.memory_hint') }}</p>
      <p v-if="memories.length === 0" class="muted">{{ t('settings.care.memory_empty') }}</p>
      <ul v-else class="items">
        <li v-for="m in memories" :key="m.id" class="item memory">
          <template v-if="editing === m.id">
            <input
              ref="editInput"
              v-model="draft"
              class="name"
              type="text"
              :maxlength="MEMORY_ENTRY_CHARS"
              :aria-label="t('settings.care.memory_edit_label')"
              @keydown.enter.prevent="saveEdit(m)"
              @keydown.esc.stop="editing = null"
            />
            <button class="compact primary" @click="saveEdit(m)">{{ t('settings.care.memory_save') }}</button>
            <button class="compact" @click="editing = null">{{ t('settings.care.memory_cancel') }}</button>
          </template>
          <template v-else>
            <span class="name">
              {{ m.content }}
              <span class="date">{{ day(m.created_ts) }}</span>
            </span>
            <button class="compact" @click="startEdit(m)">{{ t('settings.care.memory_edit') }}</button>
            <button
              class="compact"
              :class="{ danger: confirming === m.id }"
              @click="remove(m)"
              @blur="confirming === m.id && (confirming = null)"
            >
              {{
                confirming === m.id
                  ? t('settings.care.memory_delete_confirm')
                  : t('settings.care.memory_delete')
              }}
            </button>
          </template>
        </li>
      </ul>
      <p v-if="memories.length >= MEMORY_MAX" class="hint">{{ t('error.memory_full') }}</p>
      <p v-if="fieldError.memory" class="error" role="alert">{{ fieldError.memory }}</p>
    </fieldset>
  </section>
</template>

<style scoped>
.group {
  display: grid;
  gap: var(--xq-sp-2);
  max-width: 560px;
  margin: 0 0 var(--xq-sp-4);
  padding: var(--xq-sp-3) var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
}

legend {
  padding: 0 var(--xq-sp-1);
  font-weight: 600;
}

.count {
  margin-left: var(--xq-sp-2);
  color: var(--xq-text-3);
  font-size: var(--xq-fs-sm);
  font-weight: 400;
}

.choices {
  display: flex;
  flex-wrap: wrap;
  gap: 0 var(--xq-sp-4);
}

/* 整行都能点，点击目标不小于 32 px 高（DS-A11Y-03） */
.choice {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  min-height: 32px;
  cursor: pointer;
}

.choice.style {
  align-items: baseline;
}

.eg {
  margin-left: var(--xq-sp-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.hint {
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.note {
  margin: 0;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  font-size: var(--xq-fs-sm);
}

input[type='text'],
input[type='tel'],
input[type='time'] {
  min-height: 32px;
  padding: 0 var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  color: var(--xq-text-1);
  font: inherit;
}

.items {
  margin: 0;
  padding: 0;
  list-style: none;
}

.item {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  min-height: 36px;
  padding: var(--xq-sp-1) 0 var(--xq-sp-1) var(--xq-sp-2);
  border-bottom: 1px solid var(--xq-border);
}

.item:last-child {
  border-bottom: none;
}

.name {
  flex: 1;
  min-width: 0;
  overflow-wrap: anywhere;
}

.date {
  margin-left: var(--xq-sp-2);
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
}

.add {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
}

.add input[type='text'] {
  flex: 1;
}

.inline {
  display: flex;
  gap: var(--xq-sp-1);
  align-items: center;
}

.phone {
  max-width: 240px;
}

.compact {
  padding: 0 var(--xq-sp-3);
}

.icon {
  padding: 0;
  border-color: transparent;
  background: transparent;
  color: var(--xq-text-2);
}

.icon path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
}

.danger {
  border-color: var(--xq-danger);
  color: var(--xq-danger);
}

.error {
  margin: 0;
  color: var(--xq-danger);
}
</style>
