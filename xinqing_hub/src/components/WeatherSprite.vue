<script setup lang="ts">
// 天气小精灵（16 D-03）。lottie 素材到位前用 SVG + CSS 动画（16 第 3 节允许先占位）：
// - 每种天气一段 2–4 秒的循环（DS-MOTION-02）：呼吸、眨眼，加上各自配饰的动作；
// - 一次性动作 play('shake')“晃一下”（打错字，0.5 秒）、play('approach')“靠近”（出现暖心话，0.6 秒）；
//   “闭眼”就是 eyesClosed（无痕 / 密码框 / 暂停）；
// - 系统关闭动画时 base.css 把时长归零，每段关键帧的首尾都是静止姿态，所以只剩静态帧（DS-MOTION-03）；
//   窗口不可见时暂停（DS-MOTION-05）；闪电每 3 秒闪一下，远低于每秒 3 次（DS-MOTION-04）。
// 状态必须同时有形状和文字（DS-COLOR-02），这里每种天气都有不同的配饰形状。
import { computed, ref } from 'vue'
import type { Weather } from '@/api'
import { t } from '@/i18n'
import { weatherColor, weatherName } from '@/weather'

const props = withDefaults(defineProps<{ weather: Weather; eyesClosed?: boolean; size?: number }>(), {
  eyesClosed: false,
  size: 96,
})

const label = computed(() => t('widget.sprite_label', { weather: weatherName(props.weather) }))

export type SpriteAction = 'shake' | 'approach'

/** 正在播放的一次性动作；动画结束（animationend）时清空，再次 play 同一个动作会重新开始 */
const action = ref<SpriteAction | null>(null)

function play(a: SpriteAction): void {
  action.value = null
  // 先去掉再加上 class，浏览器才会从头播放
  requestAnimationFrame(() => (action.value = a))
}

function onAnimationEnd(e: AnimationEvent): void {
  if (e.animationName.startsWith('xq-once-')) action.value = null
}

defineExpose({ play })
</script>

<template>
  <svg
    class="sprite"
    :class="action && `is-${action}`"
    :width="size"
    :height="size"
    viewBox="0 0 96 96"
    role="img"
    :aria-label="label"
    :data-weather="weather"
    @animationend="onAnimationEnd"
  >
    <g class="figure">
      <g class="accessory">
        <g v-if="weather === 'sunny'" class="rays">
          <line
            v-for="i in 8"
            :key="i"
            x1="48"
            y1="4"
            x2="48"
            y2="12"
            :transform="`rotate(${i * 45} 48 48)`"
          />
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
      <g class="breath">
        <circle class="body" cx="48" cy="50" r="30" :fill="weatherColor(weather)" />
        <g class="face">
          <g v-if="eyesClosed" class="eyes closed">
            <path d="M34 48q4 4 8 0" />
            <path d="M54 48q4 4 8 0" />
          </g>
          <g v-else class="eyes open">
            <circle cx="38" cy="47" r="3" />
            <circle cx="58" cy="47" r="3" />
          </g>
          <path d="M42 60q6 5 12 0" />
        </g>
      </g>
    </g>
  </svg>
</template>

<style scoped>
.sprite {
  display: block;
  flex: none;
  overflow: visible;
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

/* ---------- 循环动画（DS-MOTION-02）：关键帧首尾都是静止姿态 ---------- */

.figure,
.breath,
.eyes,
.accessory > * {
  transform-box: view-box;
}

/* 呼吸：身体和脸一起轻轻起伏 */
.breath {
  transform-origin: 48px 80px;
  animation: xq-breathe 3s var(--xq-ease) infinite;
}

/* 闭眼时呼吸放慢（休息中） */
.sprite:has(.eyes.closed) .breath {
  animation-duration: 4s;
}

/* 眨眼：4 秒一次，只在睁眼时 */
.eyes.open {
  transform-origin: 48px 47px;
  animation: xq-blink 4s linear infinite;
}

.rays {
  transform-origin: 48px 48px;
  animation: xq-rays 4s linear infinite;
}

.cloud {
  animation: xq-drift 4s var(--xq-ease) infinite;
}

.drops path {
  animation: xq-fall 2s ease-in infinite;
}

.drops path:nth-child(2) {
  animation-delay: -0.7s;
}

.drops path:nth-child(3) {
  animation-delay: -1.4s;
}

.bolt {
  animation: xq-flash 3s linear infinite;
}

.moon {
  animation: xq-bob 4s var(--xq-ease) infinite;
}

.gust path {
  animation: xq-gust 2s var(--xq-ease) infinite;
}

.gust path:nth-child(2) {
  animation-delay: -0.3s;
}

.gust path:nth-child(3) {
  animation-delay: -0.6s;
}

@keyframes xq-breathe {
  0%,
  100% {
    transform: scale(1);
  }
  50% {
    transform: scale(1.03, 0.98);
  }
}

@keyframes xq-blink {
  0%,
  92%,
  100% {
    transform: scaleY(1);
  }
  96% {
    transform: scaleY(0.1);
  }
}

/* 8 道光芒对称，转 45° 与起点看起来一样，4 秒一圈无缝衔接 */
@keyframes xq-rays {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(45deg);
  }
}

@keyframes xq-drift {
  0%,
  100% {
    transform: translateX(0);
  }
  50% {
    transform: translateX(4px);
  }
}

@keyframes xq-fall {
  0% {
    transform: translateY(-4px);
    opacity: 0;
  }
  20%,
  70% {
    opacity: 1;
  }
  100% {
    transform: translateY(0);
    opacity: 1;
  }
}

/* 3 秒闪一下（一次变暗再亮回来），远低于 DS-MOTION-04 的每秒 3 次 */
@keyframes xq-flash {
  0%,
  80%,
  90%,
  100% {
    opacity: 1;
  }
  85% {
    opacity: 0.35;
  }
}

@keyframes xq-bob {
  0%,
  100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-3px);
  }
}

@keyframes xq-gust {
  0%,
  100% {
    transform: translateX(0);
  }
  50% {
    transform: translateX(3px);
  }
}

/* ---------- 一次性动作 ---------- */

.figure {
  transform-origin: 48px 80px;
}

/* 晃一下（打错字，0.5 秒）：左右摆两下，回到原位 */
.is-shake .figure {
  animation: xq-once-shake 0.5s var(--xq-ease);
}

/* 靠近（出现暖心话，0.6 秒）：稍微放大再回来，像凑近了一点 */
.is-approach .figure {
  animation: xq-once-approach 0.6s var(--xq-ease);
}

@keyframes xq-once-shake {
  0%,
  100% {
    transform: rotate(0deg);
  }
  25% {
    transform: rotate(-6deg);
  }
  50% {
    transform: rotate(5deg);
  }
  75% {
    transform: rotate(-3deg);
  }
}

@keyframes xq-once-approach {
  0%,
  100% {
    transform: scale(1);
  }
  45% {
    transform: scale(1.08);
  }
}
</style>
