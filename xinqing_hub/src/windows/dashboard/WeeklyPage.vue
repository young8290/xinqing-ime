<script setup lang="ts">
// 看板 · 周报（07 FR-DSH-04）。现在有作息洞察（get_routine，FR-REV-03）；
// 状态分布环形图、7 × 24 热力图、每日输入时长等要等周统计命令（B / C），先说明还在准备中。
import { t } from '@/i18n'
import RoutineInsight from './RoutineInsight.vue'
</script>

<template>
  <section>
    <h1>{{ t('dashboard.weekly') }}</h1>
    <RoutineInsight />
    <p class="muted pending">{{ t('dashboard.weekly_pending') }}</p>
  </section>
</template>

<style scoped>
.pending {
  margin-top: var(--xq-sp-5);
  font-size: var(--xq-fs-sm);
}
</style>
