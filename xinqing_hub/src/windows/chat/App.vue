<script setup lang="ts">
// 对话窗口（07 FR-CHT-01，16 D-06）。骨架阶段只有常驻的 AI 说明（DS-COPY-05）和占位。
import { t } from '@/i18n'
</script>

<template>
  <main class="page">
    <p class="notice" role="note">{{ t('chat.header_notice') }}</p>
    <h1>{{ t('window.chat') }}</h1>
    <p class="muted">{{ t('common.coming_soon') }}</p>
  </main>
</template>

<style scoped>
.page {
  padding: var(--xq-sp-5);
}

.notice {
  margin: calc(-1 * var(--xq-sp-5)) calc(-1 * var(--xq-sp-5)) var(--xq-sp-5);
  padding: var(--xq-sp-2) var(--xq-sp-5);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}
</style>
