<script setup lang="ts">
// 首次引导（07 FR-ONB-01～05）：年龄确认 → 告知 → 单独同意 → 增强模式单独一页 → AI 服务（可跳过）→ 偏好。
// 每项同意都是独立复选框、默认不勾选，勾选即调用 consent_set 落库（带版本号和时间）。
// 完成后显示小组件，晴晴打招呼（firstRun.ts 交给小组件）。
import { computed, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { commands, unwrap, type ConsentItem, type ConsentState } from '@/api'
import { errorText, t } from '@/i18n'
import { CONSENT_NUMBER as NUMBER } from '../shared/consent'
import { requestGreeting } from '../shared/firstRun'
import NoticeTable from '../shared/NoticeTable.vue'
import AiStep from './AiStep.vue'
import PrefsStep from './PrefsStep.vue'

type Step = 'age' | 'minor' | 'notice' | 'consent' | 'enhanced' | 'ai' | 'prefs'

const step = ref<Step>('age')
const consent = ref<ConsentState | null>(null)
const error = ref<string | null>(null)
const busy = ref(false)

const granted = (item: ConsentItem) => consent.value?.items.some((e) => e.item === item && e.granted) ?? false
const mainItems = computed(() => consent.value?.items.filter((e) => e.item !== 'enhanced') ?? [])
const canContinue = computed(() => granted('sense'))

onMounted(async () => {
  try {
    consent.value = await unwrap(commands.consentGet())
  } catch (e) {
    error.value = errorText(e)
  }
})

async function toggle(item: ConsentItem, on: boolean): Promise<void> {
  error.value = null
  busy.value = true
  try {
    consent.value = await unwrap(commands.consentSet(item, on))
  } catch (e) {
    error.value = errorText(e)
  } finally {
    busy.value = false
  }
}

async function finish(): Promise<void> {
  requestGreeting()
  try {
    await unwrap(commands.openWindow('widget'))
    await getCurrentWindow().close()
  } catch (e) {
    error.value = errorText(e)
  }
}

function close(): void {
  void getCurrentWindow().close()
}
</script>

<template>
  <main class="page">
    <h1>{{ t('window.onboarding') }}</h1>
    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <section v-if="step === 'age'">
      <p>{{ t('onboarding.age') }}</p>
      <div class="actions">
        <button class="primary" @click="step = 'notice'">{{ t('onboarding.btn_adult') }}</button>
        <button @click="step = 'minor'">{{ t('onboarding.btn_minor') }}</button>
      </div>
    </section>

    <section v-else-if="step === 'minor'">
      <p>{{ t('onboarding.under18') }}</p>
      <div class="actions">
        <button @click="close">{{ t('onboarding.btn_close') }}</button>
      </div>
    </section>

    <section v-else-if="step === 'notice'">
      <h2>{{ t('onboarding.notice_title') }}</h2>
      <NoticeTable />
      <p>{{ t('onboarding.third_party') }}</p>
      <p class="muted">{{ t('onboarding.not_medical') }}</p>
      <div class="actions">
        <button @click="step = 'age'">{{ t('onboarding.btn_back') }}</button>
        <button class="primary" @click="step = 'consent'">{{ t('onboarding.btn_next') }}</button>
      </div>
    </section>

    <section v-else-if="step === 'consent'">
      <h2>{{ t('onboarding.consent_title') }}</h2>
      <p class="muted">{{ t('onboarding.consent_hint') }}</p>
      <label v-for="e in mainItems" :key="e.item" class="consent" :data-item="e.item">
        <input type="checkbox" :checked="e.granted" :disabled="busy" @change="toggle(e.item, !e.granted)" />
        <span>{{ NUMBER[e.item] }} {{ t(`consent.${e.item}`) }}</span>
      </label>
      <p v-if="!canContinue" class="muted">{{ t('onboarding.sense_required') }}</p>
      <div class="actions">
        <button @click="step = 'notice'">{{ t('onboarding.btn_back') }}</button>
        <button class="primary" :disabled="!canContinue" @click="step = 'enhanced'">
          {{ t('onboarding.btn_next') }}
        </button>
      </div>
    </section>

    <AiStep v-else-if="step === 'ai'" @back="step = 'enhanced'" @next="step = 'prefs'" />

    <PrefsStep v-else-if="step === 'prefs'" @back="step = 'ai'" @done="finish" />

    <section v-else>
      <!-- FR-ONB-04：增强模式单独一页，加粗提示 -->
      <label class="consent enhanced" data-item="enhanced">
        <input
          type="checkbox"
          :checked="granted('enhanced')"
          :disabled="busy"
          @change="toggle('enhanced', !granted('enhanced'))"
        />
        <strong>{{ NUMBER.enhanced }} {{ t('consent.enhanced') }}</strong>
      </label>
      <div class="actions">
        <button @click="step = 'consent'">{{ t('onboarding.btn_back') }}</button>
        <button class="primary" :disabled="!canContinue" @click="step = 'ai'">
          {{ t('onboarding.btn_next') }}
        </button>
      </div>
    </section>
  </main>
</template>

<style scoped>
.page {
  padding: var(--xq-sp-6);
}

.actions {
  display: flex;
  gap: var(--xq-sp-3);
  justify-content: flex-end;
  margin-top: var(--xq-sp-5);
}

.consent {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: flex-start;
  min-height: 32px;
  margin-bottom: var(--xq-sp-3);
}

.consent input {
  margin-top: 5px;
}

.enhanced {
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}

.error {
  color: var(--xq-danger);
}
</style>
