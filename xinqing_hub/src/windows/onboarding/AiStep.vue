<script setup lang="ts">
// 首次引导 · AI 服务（07 FR-ONB-05 第 1 条，ADR 0012）：情绪识别（Jev）和大模型各填地址与密钥，可以跳过（离线模式）。
// 密钥只进不出：已保存的只显示末 4 位，密钥框留空就沿用旧的。“测试连接”先保存再逐个模型发一条最小请求；
// Jev 没有测试接口，只说明怎么看。模型列表引导页不让改，沿用已有的（没有就用默认），完整的放设置页“AI 服务”。
import { computed, onMounted, ref } from 'vue'
import { commands, unwrap, type AiConfigView, type ModelProbeView } from '@/api'
import { errorText, t } from '@/i18n'
import { aiFormDirty, devHint as hintFor, keyPlaceholder, toAiInput } from '../shared/aiConfig'

const emit = defineEmits<{ next: []; back: [] }>()

const view = ref<AiConfigView | null>(null)
const jevUrl = ref('')
const jevKey = ref('')
const llmUrl = ref('')
const llmKey = ref('')
const busy = ref(false)
const testing = ref(false)
const results = ref<ModelProbeView[] | null>(null)
const error = ref<string | null>(null)

const form = () => ({
  jevUrl: jevUrl.value,
  jevKey: jevKey.value,
  llmUrl: llmUrl.value,
  llmKey: llmKey.value,
})
const devHint = computed(() => hintFor(view.value?.source))
/** 填过东西（或改了已保存的地址）才需要保存 */
const dirty = computed(() => aiFormDirty(form(), view.value))
const filled = computed(() => jevUrl.value.trim() !== '' || llmUrl.value.trim() !== '')

onMounted(async () => {
  try {
    const v = await unwrap(commands.aiConfigGet())
    view.value = v
    jevUrl.value = v.jev?.base_url ?? ''
    llmUrl.value = v.llm?.base_url ?? ''
  } catch (e) {
    error.value = errorText(e)
  }
})

/** 保存；失败时显示原因并返回 false。保存后密钥框清空，占位里显示新的末 4 位。 */
async function save(): Promise<boolean> {
  if (!dirty.value) return true
  error.value = null
  busy.value = true
  try {
    view.value = await unwrap(commands.secretsSet(toAiInput(form(), view.value)))
    jevKey.value = ''
    llmKey.value = ''
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

async function next(): Promise<void> {
  if (await save()) emit('next')
}
</script>

<template>
  <section>
    <h2>{{ t('onboarding.ai.title') }}</h2>
    <p class="muted">{{ t('onboarding.ai.intro') }}</p>
    <p v-if="devHint" class="dev">{{ devHint }}</p>

    <fieldset data-service="jev">
      <legend>{{ t('ai_service.jev') }}</legend>
      <label>
        <span>{{ t('ai_service.url') }}</span>
        <input v-model="jevUrl" type="url" inputmode="url" autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>{{ t('ai_service.key') }}</span>
        <input
          v-model="jevKey"
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
        <input v-model="llmUrl" type="url" inputmode="url" autocomplete="off" spellcheck="false" />
      </label>
      <label>
        <span>{{ t('ai_service.key') }}</span>
        <input
          v-model="llmKey"
          type="password"
          autocomplete="off"
          :placeholder="keyPlaceholder(view?.llm?.key_tail)"
        />
      </label>
    </fieldset>

    <div class="test">
      <button :disabled="busy || testing || !filled" @click="test">
        {{ testing ? t('ai_service.testing') : t('ai_service.test') }}
      </button>
      <ul v-if="results" class="results" role="status">
        <li v-if="results.length === 0">{{ t('ai_service.no_llm') }}</li>
        <li v-for="r in results" :key="r.model" :class="r.ok ? 'ok' : 'fail'">
          {{
            r.ok
              ? t('ai_service.test_ok', { model: r.model, ms: r.latency_ms })
              : t('ai_service.test_fail', { model: r.model, status: r.status })
          }}
        </li>
        <li v-if="jevUrl.trim()" class="muted">{{ t('ai_service.jev_untested') }}</li>
      </ul>
    </div>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <div class="actions">
      <button @click="emit('back')">{{ t('onboarding.btn_back') }}</button>
      <button class="skip" :disabled="busy" @click="emit('next')">{{ t('onboarding.ai.skip') }}</button>
      <button class="primary" :disabled="busy || testing" @click="next">
        {{ t('onboarding.btn_next') }}
      </button>
    </div>
  </section>
</template>

<style scoped>
fieldset {
  display: grid;
  gap: var(--xq-sp-2);
  margin: 0 0 var(--xq-sp-4);
  padding: var(--xq-sp-3) var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
}

legend {
  padding: 0 var(--xq-sp-1);
  font-weight: 600;
}

label {
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

.dev {
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  font-size: var(--xq-fs-sm);
}

.test {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  align-items: flex-start;
}

.results {
  flex: 1;
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.results .fail {
  color: var(--xq-danger);
}

.actions {
  display: flex;
  gap: var(--xq-sp-3);
  justify-content: flex-end;
  margin-top: var(--xq-sp-5);
}

/* “跳过”放在上一步与下一步之间，靠左的空白把“上一步”推开 */
.skip {
  margin-left: auto;
}

.error {
  color: var(--xq-danger);
}
</style>
