<script setup lang="ts">
// 告知表（07 FR-ONB-03）：用到什么、用来做什么、去哪里。首次引导与设置“隐私”共用，两处说法一致（FR-SET-09）。
import { t, type CopyKey } from '@/i18n'

const ROWS = ['rhythm', 'sentence', 'rewrite', 'summary', 'chat', 'app'] as const
const cell = (row: string, col: 'what' | 'use' | 'where') => t(`onboarding.notice.${row}.${col}` as CopyKey)
</script>

<template>
  <table class="notice">
    <thead>
      <tr>
        <th scope="col">{{ t('onboarding.notice_col_what') }}</th>
        <th scope="col">{{ t('onboarding.notice_col_use') }}</th>
        <th scope="col">{{ t('onboarding.notice_col_where') }}</th>
      </tr>
    </thead>
    <tbody>
      <tr v-for="row in ROWS" :key="row">
        <td>{{ cell(row, 'what') }}</td>
        <td>{{ cell(row, 'use') }}</td>
        <td>{{ cell(row, 'where') }}</td>
      </tr>
    </tbody>
  </table>
</template>

<style scoped>
.notice {
  width: 100%;
  margin-bottom: var(--xq-sp-4);
  border-collapse: collapse;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.notice th,
.notice td {
  padding: var(--xq-sp-2);
  border-bottom: 1px solid var(--xq-border);
  text-align: left;
  vertical-align: top;
}
</style>
