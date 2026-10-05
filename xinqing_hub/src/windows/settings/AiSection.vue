<script setup lang="ts">
// 设置中心 · AI 服务（07 FR-SET-08，ADR 0012 / 0019）：Jev 与大模型的地址和密钥（只显示末 4 位）、大模型优先级（拖动或按钮排序）、
// “测试连接”（每个模型是否可用和延迟）、每日调用上限（ai.cap.*，范围取自 settings_schema）与今日用量。
// 地址、密钥、模型三样一起点“保存”才写（输入一半时不该生效）；调用上限改了就存，网关立即换用。
import { computed, onMounted, reactive, ref } from 'vue'
import {
  commands,
  unwrap,
  type AiConfigView,
  type BudgetKind,
  type ModelProbeView,
  type SettingField,
  type UsageRow,
} from '@/api'
import { useSettingsStore } from '@/stores/settings'
import { errorText, t } from '@/i18n'
import { aiFormDirty, devHint, keyPlaceholder, toAiInput } from '../shared/aiConfig'

/** 用量的种类 → 调用上限的设置键（ADR 0019 第 4 条） */
const CAP_KEY: Record<BudgetKind, string> = {
  jev: 'ai.cap.jev',
  llm: 'ai.cap.llm',
  chat_turn: 'ai.cap.chat',
  schedule_prefilter: 'ai.cap.schedule',
  rewrite: 'ai.cap.rewrite',
}
const KINDS = Object.keys(CAP_KEY) as BudgetKind[]

const settings = useSettingsStore()
const view = ref<AiConfigView | null>(null)
const form = reactive({ jevUrl: '', jevKey: '', llmUrl: '', llmKey: '', models: [] as string[] })
const newModel = ref('')
const dragging = ref<number | null>(null)
const busy = ref(false)
const saved = ref(false)
const testing = ref(false)
const results = ref<ModelProbeView[] | null>(null)
const usage = ref<UsageRow[]>([])
const ranges = ref<Record<string, { min: number | null; max: number | null }>>({})
const error = ref<string | null>(null)
const capError = ref<string | null>(null)
let savedTimer: ReturnType<typeof setTimeout> | undefined

const dirty = computed(() => aiFormDirty(form, view.value))
const hint = computed(() => devHint(view.value?.source))
const offline = computed(() => view.value?.source === 'none')

function fill(v: AiConfigView): void {
  view.value = v
  form.jevUrl = v.jev?.base_url ?? ''
  form.llmUrl = v.llm?.base_url ?? ''
  form.jevKey = ''
  form.llmKey = ''
  form.models = [...(v.llm?.models ?? [])]
}

async function loadUsage(): Promise<void> {
  usage.value = await unwrap(commands.aiUsageToday())
}

onMounted(async () => {
  try {
    const [v, schema] = await Promise.all([unwrap(commands.aiConfigGet()), commands.settingsSchema()])
    fill(v)
    ranges.value = Object.fromEntries(
      schema
        .filter((f: SettingField) => f.kind.type === 'int' && f.key.startsWith('ai.cap.'))
        .map((f) => [
          f.key,
          f.kind.type === 'int' ? { min: f.kind.min, max: f.kind.max } : { min: null, max: null },
        ]),
    )
    await Promise.all([loadUsage(), ...KINDS.map((k) => settings.load(CAP_KEY[k]))])
  } catch (e) {
    error.value = errorText(e)
  }
})

// ── 大模型优先级 ──
function addModel(): void {
  const m = newModel.value.trim()
  if (m === '' || form.models.includes(m)) return
  form.models.push(m)
  newModel.value = ''
}

function move(from: number, to: number): void {
  if (to < 0 || to >= form.models.length || from === to) return
  const [m] = form.models.splice(from, 1)
  form.models.splice(to, 0, m!)
}

function onDrop(to: number): void {
  if (dragging.value !== null) move(dragging.value, to)
  dragging.value = null
}

// ── 保存与测试 ──
async function save(): Promise<boolean> {
  if (!dirty.value) return true
  error.value = null
  busy.value = true
  try {
    fill(await unwrap(commands.secretsSet(toAiInput(form, view.value))))
    saved.value = true
    clearTimeout(savedTimer)
    savedTimer = setTimeout(() => (saved.value = false), 2000)
    return true
  } catch (e) {
    error.value = errorText(e)
    return false
  } finally {
    busy.value = false
  }
}

async function test(): Promise<void> {
  if (!(await save())) return
  testing.value = true
  results.value = null
  try {
    results.value = await unwrap(commands.aiTestConnection())
  } catch (e) {
    error.value = errorText(e)
  } finally {
    testing.value = false
  }
}

// ── 调用上限与今日用量 ──
function capOf(kind: BudgetKind): number | null {
  const v = settings.values[CAP_KEY[kind]]
  return typeof v === 'number' ? v : null
}

function usedOf(kind: BudgetKind): number {
  return usage.value.find((u) => u.kind === kind)?.used ?? 0
}

/** 今日用量占上限的百分比（进度条用，0–100） */
function pct(kind: BudgetKind): number {
  const cap = capOf(kind) ?? 0
  return cap > 0 ? Math.min(100, Math.round((usedOf(kind) / cap) * 100)) : 0
}

async function setCap(kind: BudgetKind, raw: string): Promise<void> {
  const n = Math.round(Number(raw))
  if (raw.trim() === '' || !Number.isFinite(n) || n === capOf(kind)) return
  capError.value = null
  try {
    await settings.set(CAP_KEY[kind], n)
    await loadUsage()
  } catch (e) {
    capError.value = errorText(e)
  }
}
</script>

<template>
  <section>
    <h1>{{ t('settings.ai_service') }}</h1>
    <p v-if="hint" class="note">{{ hint }}</p>
    <p v-else-if="offline" class="note">{{ t('settings.ai.offline') }}</p>

    <fieldset data-service="jev">
      <legend>{{ t('ai_service.jev') }}</legend>
      <label>
        <span>{{ t('ai_service.url') }}</span>
        <input v-model="form.jevUrl" type="url" autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>{{ t('ai_service.key') }}</span>
        <input
          v-model="form.jevKey"
          type="password"
          autocomplete="off"
          :placeholder="keyPlaceholder(view?.jev?.key_tail)"
        />
      </label>
    </fieldset>

    <fieldset data-service="llm">
      <legend>{{ t('ai_service.llm') }}</legend>
      <label>
        <span>{{ t('ai_service.url') }}</span>
        <input v-model="form.llmUrl" type="url" autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>{{ t('ai_service.key') }}</span>
        <input
          v-model="form.llmKey"
          type="password"
          autocomplete="off"
          :placeholder="keyPlaceholder(view?.llm?.key_tail)"
        />
      </label>
      <div class="models">
        <span class="models-title">{{ t('settings.ai.models') }}</span>
        <ol>
          <li
            v-for="(m, i) in form.models"
            :key="m"
            class="model"
            :class="{ dragging: dragging === i }"
            draggable="true"
            @dragstart="dragging = i"
            @dragend="dragging = null"
            @dragover.prevent
            @drop.prevent="onDrop(i)"
          >
            <svg class="grip" width="10" height="14" viewBox="0 0 10 14" aria-hidden="true">
              <circle cx="3" cy="3" r="1.2" />
              <circle cx="7" cy="3" r="1.2" />
              <circle cx="3" cy="7" r="1.2" />
              <circle cx="7" cy="7" r="1.2" />
              <circle cx="3" cy="11" r="1.2" />
              <circle cx="7" cy="11" r="1.2" />
            </svg>
            <span class="name">{{ m }}</span>
            <button
              class="icon"
              :disabled="i === 0"
              :aria-label="t('settings.ai.model_up', { model: m })"
              @click="move(i, i - 1)"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                <path d="M3 7.5 6 4.5l3 3" />
              </svg>
            </button>
            <button
              class="icon"
              :disabled="i === form.models.length - 1"
              :aria-label="t('settings.ai.model_down', { model: m })"
              @click="move(i, i + 1)"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                <path d="M3 4.5 6 7.5l3-3" />
              </svg>
            </button>
            <button
              class="icon"
              :aria-label="t('settings.ai.model_remove', { model: m })"
              @click="form.models.splice(i, 1)"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
                <path d="M3 3l6 6M9 3l-6 6" />
              </svg>
            </button>
          </li>
        </ol>
        <div class="add">
          <input
            v-model="newModel"
            type="text"
            :aria-label="t('settings.ai.model_add')"
            :placeholder="t('settings.ai.model_add')"
            @keydown.enter.prevent="addModel"
          />
          <button class="compact" @click="addModel">{{ t('ime.list_add') }}</button>
        </div>
        <p class="muted small">{{ t('settings.ai.models_hint') }}</p>
      </div>
    </fieldset>

    <div class="row-actions">
      <button class="primary" :disabled="busy || !dirty" @click="save">{{ t('settings.ai.save') }}</button>
      <button :disabled="busy || testing || (!form.llmUrl.trim() && !form.jevUrl.trim())" @click="test">
        {{ testing ? t('ai_service.testing') : t('ai_service.test') }}
      </button>
      <span v-if="saved" class="saved" role="status">{{ t('settings.saved') }}</span>
    </div>
    <ul v-if="results" class="results" role="status">
      <li v-if="results.length === 0">{{ t('ai_service.no_llm') }}</li>
      <li v-for="r in results" :key="r.model" :class="r.ok ? 'ok' : 'fail'">
        {{
          r.ok
            ? t('ai_service.test_ok', { model: r.model, ms: r.latency_ms })
            : t('ai_service.test_fail', { model: r.model, status: r.status })
        }}
      </li>
      <li v-if="form.jevUrl.trim()" class="muted">{{ t('ai_service.jev_untested') }}</li>
    </ul>
    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <h2>{{ t('settings.ai.caps') }}</h2>
    <p class="muted small">{{ t('settings.ai.caps_hint') }}</p>
    <div v-for="k in KINDS" :key="k" class="cap" :data-kind="k">
      <label :for="`cap-${k}`">{{ t(`settings.ai.kind_${k}`) }}</label>
      <input
        :id="`cap-${k}`"
        type="number"
        step="1"
        :min="ranges[CAP_KEY[k]]?.min ?? 0"
        :max="ranges[CAP_KEY[k]]?.max ?? undefined"
        :value="capOf(k) ?? ''"
        @change="setCap(k, ($event.target as HTMLInputElement).value)"
      />
      <span class="usage">
        <span class="bar" aria-hidden="true"><span :style="{ width: `${pct(k)}%` }" /></span>
        {{ t('settings.ai.usage', { used: usedOf(k), cap: capOf(k) ?? 0 }) }}
      </span>
    </div>
    <p v-if="capError" class="error" role="alert">{{ capError }}</p>
  </section>
</template>

<style scoped>
h2 {
  margin-top: var(--xq-sp-5);
}

fieldset {
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

fieldset > label {
  display: grid;
  grid-template-columns: 72px 1fr;
  gap: var(--xq-sp-3);
  align-items: center;
}

input {
  min-height: 32px;
  padding: 0 var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  color: var(--xq-text-1);
  font: inherit;
}

.note {
  max-width: 560px;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  font-size: var(--xq-fs-sm);
}

.models-title {
  display: block;
  margin-top: var(--xq-sp-2);
}

.models ol {
  margin: var(--xq-sp-1) 0;
  padding: 0;
  list-style: none;
}

.model {
  display: flex;
  gap: var(--xq-sp-1);
  align-items: center;
  padding-left: var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface);
  cursor: grab;
}

.model + .model {
  margin-top: var(--xq-sp-1);
}

.model.dragging {
  opacity: 0.5;
}

.grip {
  flex: none;
  fill: var(--xq-text-3);
}

.name {
  flex: 1;
  overflow-wrap: anywhere;
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
  stroke-linejoin: round;
}

.add {
  display: flex;
  gap: var(--xq-sp-2);
}

.add input {
  flex: 1;
}

.compact {
  padding: 0 var(--xq-sp-3);
}

.small {
  margin: var(--xq-sp-1) 0 0;
  font-size: var(--xq-fs-sm);
}

.row-actions {
  display: flex;
  gap: var(--xq-sp-3);
  align-items: center;
}

.saved {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.results {
  margin: var(--xq-sp-3) 0 0;
  padding: 0;
  list-style: none;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.results .fail,
.error {
  color: var(--xq-danger);
}

.cap {
  display: grid;
  grid-template-columns: 160px 120px 1fr;
  gap: var(--xq-sp-3);
  align-items: center;
  max-width: 560px;
  padding: var(--xq-sp-2) 0;
  border-bottom: 1px solid var(--xq-border);
}

.usage {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

.bar {
  flex: none;
  width: 64px;
  height: 6px;
  overflow: hidden;
  border-radius: 3px;
  background: var(--xq-surface-2);
}

.bar span {
  display: block;
  height: 100%;
  background: var(--xq-primary);
}
</style>
