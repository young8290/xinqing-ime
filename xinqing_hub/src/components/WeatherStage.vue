<script setup lang="ts">
// 天气切换时新旧小精灵 400 ms 交叉淡化（07 FR-WGT-03、DS-MOTION-01）。
// 只有天气变了才淡化；闭眼、睁眼在同一个小精灵上直接切换。系统关闭动画时 base.css 把时长归零。
import type { Weather } from '@/api'
import WeatherSprite from './WeatherSprite.vue'

withDefaults(defineProps<{ weather: Weather; eyesClosed?: boolean; size?: number }>(), {
  eyesClosed: false,
  size: 96,
})
</script>

<template>
  <div class="stage" :style="{ width: `${size}px`, height: `${size}px` }">
    <Transition name="xq-crossfade">
      <WeatherSprite :key="weather" :weather="weather" :eyes-closed="eyesClosed" :size="size" />
    </Transition>
  </div>
</template>

<style scoped>
.stage {
  position: relative;
  flex: none;
}

.stage > * {
  position: absolute;
  inset: 0;
}

.xq-crossfade-enter-active,
.xq-crossfade-leave-active {
  transition: opacity var(--xq-dur-weather) var(--xq-ease);
}

.xq-crossfade-enter-from,
.xq-crossfade-leave-to {
  opacity: 0;
}
</style>
