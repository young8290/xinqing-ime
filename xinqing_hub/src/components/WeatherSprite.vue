<script setup lang="ts">
// 天气小精灵的静态占位（16 D-03：lottie 动画到位前先用静态 SVG）。
// 状态必须同时有形状和文字（DS-COLOR-02），这里每种天气都有不同的配饰形状。
import { computed } from 'vue'
import type { Weather } from '@/api'
import { t } from '@/i18n'
import { weatherColor, weatherName } from '@/weather'

const props = withDefaults(defineProps<{ weather: Weather; eyesClosed?: boolean; size?: number }>(), {
  eyesClosed: false,
  size: 96,
})

const label = computed(() => t('widget.sprite_label', { weather: weatherName(props.weather) }))
</script>

<template>
  <svg
    class="sprite"
    :width="size"
    :height="size"
    viewBox="0 0 96 96"
    role="img"
    :aria-label="label"
    :data-weather="weather"
  >
    <g class="accessory">
      <g v-if="weather === 'sunny'" class="rays">
        <line v-for="i in 8" :key="i" x1="48" y1="4" x2="48" y2="12" :transform="`rotate(${i * 45} 48 48)`" />
      </g>
      <path
        v-else-if="weather === 'cloudy'"
        class="cloud"
        d="M8 34a9 9 0 0 1 9-9 12 12 0 0 1 22 2 8 8 0 0 1 0 16H17a9 9 0 0 1-9-9z"
      />
      <g v-else-if="weather === 'rain'" class="drops">
        <path d="M22 80q-3 5 0 7t0-7z" />
        <path d="M48 84q-3 5 0 7t0-7z" />
        <path d="M74 80q-3 5 0 7t0-7z" />
      </g>
      <path v-else-if="weather === 'storm'" class="bolt" d="M76 6 64 28h9l-7 18 18-24h-9l6-16z" />
      <path v-else-if="weather === 'night'" class="moon" d="M84 10a12 12 0 1 0 6 20 10 10 0 1 1-6-20z" />
      <g v-else-if="weather === 'wind'" class="gust">
        <path d="M4 30h20a5 5 0 1 0-5-5" />
        <path d="M4 40h28" />
        <path d="M4 50h18a5 5 0 1 1-5 5" />
      </g>
    </g>
    <circle class="body" cx="48" cy="50" r="30" :fill="weatherColor(weather)" />
    <g class="face">
      <template v-if="eyesClosed">
        <path d="M34 48q4 4 8 0" />
        <path d="M54 48q4 4 8 0" />
      </template>
      <template v-else>
        <circle cx="38" cy="47" r="3" />
        <circle cx="58" cy="47" r="3" />
      </template>
      <path d="M42 60q6 5 12 0" />
    </g>
  </svg>
</template>

<style scoped>
.sprite {
  display: block;
  flex: none;
}

.body {
  stroke: var(--xq-on-weather);
  stroke-width: 1.5;
  transition: fill var(--xq-dur-weather) var(--xq-ease);
}

.face circle {
  fill: var(--xq-on-weather);
}

.face path,
.accessory line,
.accessory .gust path {
  fill: none;
  stroke: var(--xq-on-weather);
  stroke-width: 2;
  stroke-linecap: round;
}

/* 光芒和风线画在身体外面、直接落在窗口底色上，要跟随深浅色 */
.accessory line,
.accessory .gust path {
  stroke: var(--xq-text-2);
}

.cloud,
.drops path,
.bolt,
.moon {
  fill: v-bind('weatherColor(weather)');
  stroke: var(--xq-on-weather);
  stroke-width: 1.5;
  stroke-linejoin: round;
}
</style>
