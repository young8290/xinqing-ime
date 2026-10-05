<script setup lang="ts">
// 首次引导 · 偏好（07 FR-ONB-05 第 2 条）：小组件放在哪个角、晴晴主动关心的频率（care.level）、四种休息提醒的开关（rest.*.enabled）。
// 改了就保存：设置键经 settings_set 落库，小组件的角存本机（firstRun.ts）。默认值来自注册表，引导页不另定默认。
import { onMounted, ref } from 'vue'
import { useSettingsStore } from '@/stores/settings'
import { errorText, t } from '@/i18n'
import { CORNERS, loadCorner, saveCorner, type Corner } from '../shared/firstRun'

const emit = defineEmits<{ done: []; back: [] }>()

const CARE = ['more', 'normal', 'less', 'off'] as const
const REST = ['eye', 'water', 'move', 'night'] as const

const settings = useSettingsStore()
const corner = ref<Corner>(loadCorner())
const error = ref<string | null>(null)

onMounted(async () => {
  try {
    await Promise.all(['care.level', ...REST.map((r) => `rest.${r}.enabled`)].map((k) => settings.load(k)))
  } catch (e) {
    error.value = errorText(e)
  }
})

function pickCorner(c: Corner): void {
  corner.value = c
  saveCorner(c)
}

async function set(key: string, value: string | boolean): Promise<void> {
  error.value = null
  try {
    await settings.set(key, value)
  } catch (e) {
    error.value = errorText(e)
  }
}

const careLevel = () => settings.values['care.level'] ?? 'normal'
const restOn = (r: (typeof REST)[number]) => settings.values[`rest.${r}.enabled`] === true
</script>

<template>
  <section>
    <h2>{{ t('onboarding.prefs.title') }}</h2>
    <p class="muted">{{ t('onboarding.prefs.hint') }}</p>

    <fieldset class="choices" data-pref="corner">
      <legend>{{ t('onboarding.prefs.corner') }}</legend>
      <label v-for="c in CORNERS" :key="c" class="choice">
        <input type="radio" name="corner" :value="c" :checked="corner === c" @change="pickCorner(c)" />
        <span>{{ t(`onboarding.prefs.corner_${c}`) }}</span>
      </label>
    </fieldset>

    <fieldset class="choices" data-pref="care">
      <legend>{{ t('onboarding.prefs.care') }}</legend>
      <label v-for="c in CARE" :key="c" class="choice">
        <input
          type="radio"
          name="care"
          :value="c"
          :checked="careLevel() === c"
          @change="set('care.level', c)"
        />
        <span>{{ t(`care_level.${c}`) }}</span>
      </label>
    </fieldset>

    <fieldset class="choices" data-pref="rest">
      <legend>{{ t('onboarding.prefs.rest') }}</legend>
      <label v-for="r in REST" :key="r" class="choice">
        <input
          type="checkbox"
          :checked="restOn(r)"
          @change="set(`rest.${r}.enabled`, ($event.target as HTMLInputElement).checked)"
        />
        <span>{{ t(`onboarding.prefs.rest_${r}`) }}</span>
      </label>
    </fieldset>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <div class="actions">
      <button @click="emit('back')">{{ t('onboarding.btn_back') }}</button>
      <button class="primary" @click="emit('done')">{{ t('onboarding.btn_done') }}</button>
    </div>
  </section>
</template>

<style scoped>
.choices {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2) var(--xq-sp-4);
  margin: 0 0 var(--xq-sp-4);
  padding: var(--xq-sp-3) var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
}

legend {
  padding: 0 var(--xq-sp-1);
  font-weight: 600;
}

/* 整行都能点，点击目标不小于 32 px 高（DS-A11Y-03） */
.choice {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  min-height: 32px;
  cursor: pointer;
}

.actions {
  display: flex;
  gap: var(--xq-sp-3);
  justify-content: flex-end;
  margin-top: var(--xq-sp-5);
}

.error {
  color: var(--xq-danger);
}
</style>
