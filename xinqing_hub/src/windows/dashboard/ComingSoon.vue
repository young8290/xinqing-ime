<script setup lang="ts">
// 看板里还没有数据来源的页：只显示页名和“还在准备中”。
import { t, type CopyKey } from '@/i18n'

const props = defineProps<{ title: CopyKey }>()
</script>

<template>
  <section>
    <h1>{{ t(props.title) }}</h1>
    <p class="muted">{{ t('dashboard.coming') }}</p>
  </section>
</template>
