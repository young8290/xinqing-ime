<script setup lang="ts">
// 天气切换时新旧小精灵 400 ms 交叉淡化（07 FR-WGT-03、DS-MOTION-01）。
// 只有天气变了才淡化；闭眼、睁眼在同一个小精灵上直接切换。系统关闭动画时 base.css 把时长归零。
// play() 转给当前这只小精灵（“晃一下”“靠近”，DS-MOTION-02）。
import { ref } from 'vue'
import type { Weather } from '@/api'
import WeatherSprite, { type SpriteAction } from './WeatherSprite.vue'

withDefaults(defineProps<{ weather: Weather; eyesClosed?: boolean; size?: number }>(), {
  eyesClosed: false,
  size: 96,
})

const sprite = ref<InstanceType<typeof WeatherSprite> | null>(null)

defineExpose({ play: (a: SpriteAction) => sprite.value?.play(a) })
</script>

<template>
  <div class="stage" :style="{ width: `${size}px`, height: `${size}px` }">
    <Transition name="xq-crossfade">
      <WeatherSprite :key="weather" ref="sprite" :weather="weather" :eyes-closed="eyesClosed" :size="size" />
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
