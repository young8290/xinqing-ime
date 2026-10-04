<script setup lang="ts">
// 情绪看板（07 FR-DSH-01～06，16 D-07）。骨架阶段只显示当前天气，验证状态快照与事件接通。
import { computed, onMounted } from 'vue'
import WeatherSprite from '@/components/WeatherSprite.vue'
import { t } from '@/i18n'
import { useStatusStore } from '@/stores/status'
import { WEATHER_ICON, hedge, weatherName } from '@/weather'

const status = useStatusStore()
const now = computed(() => {
  const s = status.snapshot
  return s && `${WEATHER_ICON[s.weather]} ${weatherName(s.weather)} · ${hedge(s.state)}`
})

onMounted(() => status.init().catch((e) => console.warn('读取状态失败', e)))
</script>

<template>
  <main class="page">
    <h1>{{ t('window.dashboard') }}</h1>
    <section v-if="status.snapshot" class="now">
      <WeatherSprite :weather="status.snapshot.weather" :size="64" />
      <p>{{ now }}</p>
    </section>
    <p class="muted">{{ t('common.coming_soon') }}</p>
  </main>
</template>

<style scoped>
.page {
  padding: var(--xq-sp-5);
}

.now {
  display: flex;
  gap: var(--xq-sp-4);
  align-items: center;
  margin-bottom: var(--xq-sp-4);
  padding: var(--xq-sp-4);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-card);
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-card);
}

.now p {
  margin: 0;
  font-size: var(--xq-fs-lg);
  line-height: var(--xq-lh-lg);
}
</style>
