<script setup lang="ts">
// 设置中心（07 FR-SET-01～10，16 D-08）。骨架阶段只有“外观”：颜色模式与小组件不透明度，
// 用来验证 settings_get / settings_set / settings:changed 一路接通。
import { computed, onMounted, ref } from 'vue'
import { errorText, t, type CopyKey } from '@/i18n'
import { useSettingsStore } from '@/stores/settings'

const THEMES = [
  ['system', 'settings.theme_system'],
  ['light', 'settings.theme_light'],
  ['dark', 'settings.theme_dark'],
] as const satisfies readonly (readonly [string, CopyKey])[]

const settings = useSettingsStore()
const error = ref<string | null>(null)
const theme = computed(() => settings.values['ui.theme'] ?? 'system')
const opacityPct = computed(() => {
  const v = settings.values['widget.opacity']
  return Math.round((typeof v === 'number' ? v : 1) * 100)
})

onMounted(async () => {
  try {
    await settings.init(['ui.theme', 'widget.opacity'])
  } catch (e) {
    error.value = errorText(e)
  }
})

async function save(key: string, value: string | number): Promise<void> {
  error.value = null
  try {
    await settings.set(key, value)
  } catch (e) {
    error.value = errorText(e)
  }
}
</script>

<template>
  <div class="layout">
    <nav class="nav">
      <a class="active" aria-current="page">{{ t('settings.appearance') }}</a>
    </nav>
    <main class="page">
      <h1>{{ t('settings.appearance') }}</h1>
      <p v-if="error" class="error" role="alert">{{ error }}</p>

      <fieldset class="field">
        <legend>{{ t('settings.theme') }}</legend>
        <label v-for="[value, key] in THEMES" :key="value" class="choice">
          <input
            type="radio"
            name="theme"
            :value="value"
            :checked="theme === value"
            @change="save('ui.theme', value)"
          />
          {{ t(key) }}
        </label>
      </fieldset>

      <label class="field">
        <span>{{ t('settings.widget_opacity') }}</span>
        <input
          type="range"
          min="70"
          max="100"
          step="5"
          :value="opacityPct"
          @change="save('widget.opacity', Number(($event.target as HTMLInputElement).value) / 100)"
        />
        <output>{{ opacityPct }}%</output>
      </label>
    </main>
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  height: 100%;
}

.nav {
  flex: none;
  width: 200px;
  padding: var(--xq-sp-4) var(--xq-sp-3);
  border-right: 1px solid var(--xq-border);
  background: var(--xq-surface-2);
}

.nav a {
  display: block;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  color: var(--xq-text-1);
  text-decoration: none;
}

.nav a.active {
  background: var(--xq-surface);
}

.page {
  flex: 1;
  padding: var(--xq-sp-5);
  overflow: auto;
}

.field {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  align-items: center;
  margin: 0 0 var(--xq-sp-5);
  padding: 0;
  border: 0;
}

.field legend {
  margin-bottom: var(--xq-sp-2);
  padding: 0;
}

.choice {
  display: inline-flex;
  gap: var(--xq-sp-1);
  align-items: center;
  min-height: 32px;
}

.error {
  color: var(--xq-danger);
}
</style>
