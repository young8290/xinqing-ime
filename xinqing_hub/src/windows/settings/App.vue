<script setup lang="ts">
// 设置中心（07 FR-SET-01～10，16 D-08）：左侧分类、右侧内容。目前有“输入法”（FR-SET-02，ADR 0016）、“外观”、
// “关怀”（FR-SET-05）、“AI 服务”（FR-SET-08）和“隐私与关于”（只有“关于”部分）；
// 其余分类随 schema 表单生成器（D-08）补上，顺序按 FR-SET-01。地址栏的 #about 直接打开“隐私与关于”。
import { ref, type Component } from 'vue'
import { t, type CopyKey } from '@/i18n'
import AboutSection from './AboutSection.vue'
import AiSection from './AiSection.vue'
import AppearanceSection from './AppearanceSection.vue'
import CareSection from './CareSection.vue'
import ImeSection from './ImeSection.vue'

type SectionId = 'ime' | 'appearance' | 'care' | 'ai' | 'privacy_about'

const SECTIONS: { id: SectionId; title: CopyKey; component: Component }[] = [
  { id: 'ime', title: 'settings.ime', component: ImeSection },
  { id: 'appearance', title: 'settings.appearance', component: AppearanceSection },
  { id: 'care', title: 'settings.care_nav', component: CareSection },
  { id: 'ai', title: 'settings.ai_service', component: AiSection },
  { id: 'privacy_about', title: 'settings.privacy_about', component: AboutSection },
]

// 托盘“设置”打开的就是 Hub（心晴不带清风的设置程序），所以默认先看“输入法”
const current = ref<SectionId>(location.hash === '#about' ? 'privacy_about' : 'ime')
</script>

<template>
  <div class="layout">
    <nav class="nav">
      <button
        v-for="s in SECTIONS"
        :key="s.id"
        class="nav-item"
        :class="{ active: current === s.id }"
        :aria-current="current === s.id ? 'page' : undefined"
        @click="current = s.id"
      >
        {{ t(s.title) }}
      </button>
    </nav>
    <main class="page">
      <component :is="SECTIONS.find((s) => s.id === current)!.component" />
    </main>
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  height: 100%;
}

.nav {
  display: flex;
  flex: none;
  flex-direction: column;
  gap: var(--xq-sp-1);
  width: 200px;
  padding: var(--xq-sp-4) var(--xq-sp-3);
  border-right: 1px solid var(--xq-border);
  background: var(--xq-surface-2);
}

.nav-item {
  justify-content: flex-start;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-color: transparent;
  background: transparent;
  text-align: left;
}

.nav-item.active {
  background: var(--xq-surface);
}

.page {
  flex: 1;
  padding: var(--xq-sp-5);
  overflow: auto;
}
</style>
