<script setup lang="ts">
// 求助卡片（05 FR-SAF-02）：固定文案，存在本地，断网照常显示。固定在对话窗口顶部；点“我现在是安全的”后折叠成一行，
// 本次会话内仍可见、不可移除。电话号码可以一键复制。“我说的不是这个意思”（FR-SAF-06）只在安全模式下出现。
import { computed, ref } from 'vue'
import { t } from '@/i18n'

const props = defineProps<{ collapsed: boolean; dismissed: boolean }>()
const emit = defineEmits<{ safe: []; expand: []; misread: [] }>()

/** 学校心理中心电话的设置项还没有（设置键注册表暂无文本类型），先显示“可在设置中添加”。 */
const lines = computed(() => t('safety.card', { school_phone: t('safety.school_phone_empty') }).split('\n'))
const NUMBERS = ['12356', '110', '120'] as const
const copied = ref<string | null>(null)
const learnMore = ref(false)

async function copyNumber(n: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(n)
    copied.value = n
  } catch (e) {
    console.warn('复制号码失败', e)
  }
}
</script>

<template>
  <section v-if="props.collapsed" class="safety collapsed" role="note">
    <p class="line">{{ t('safety.collapsed') }}</p>
    <button class="compact" @click="emit('expand')">{{ t('safety.btn_expand') }}</button>
  </section>
  <section v-else class="safety" role="alert" :aria-label="lines[0]">
    <p class="lead">{{ lines[0] }}</p>
    <ul>
      <li v-for="(l, i) in lines.slice(1)" :key="i">{{ l }}</li>
    </ul>
    <div class="numbers">
      <button v-for="n in NUMBERS" :key="n" class="compact" @click="copyNumber(n)">
        {{ copied === n ? t('safety.copied', { number: n }) : t('safety.copy_number', { number: n }) }}
      </button>
    </div>
    <p v-if="learnMore" class="learn">{{ t('safety.learn_more') }}</p>
    <div class="actions">
      <button class="compact primary" @click="emit('safe')">{{ t('safety.btn_safe') }}</button>
      <button v-if="!props.dismissed" class="compact" @click="emit('misread')">
        {{ t('safety.btn_misread') }}
      </button>
      <button class="compact link" :aria-expanded="learnMore" @click="learnMore = !learnMore">
        {{ t('safety.btn_learn_more') }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.safety {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-2);
  margin: 0 var(--xq-sp-3) var(--xq-sp-2);
  padding: var(--xq-sp-3);
  border: 1px solid var(--xq-primary);
  border-radius: var(--xq-radius-card);
  background: var(--xq-surface);
  box-shadow: var(--xq-shadow-card);
}

.collapsed {
  flex-direction: row;
  align-items: center;
  padding: var(--xq-sp-1) var(--xq-sp-2) var(--xq-sp-1) var(--xq-sp-3);
}

.line {
  flex: 1;
  margin: 0;
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.lead {
  margin: 0;
  font-weight: 600;
}

ul {
  margin: 0;
  padding: 0;
  list-style: none;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.learn {
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.numbers,
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-1);
}

/* 仍守 32 × 32 的点击目标（DS-A11Y-03），只收窄左右留白；求助卡片上的按钮不用最小字号 */
.compact {
  padding: 0 var(--xq-sp-2);
  font-size: var(--xq-fs-sm);
}

.link {
  border-color: transparent;
  background: transparent;
  color: var(--xq-link);
}
</style>
