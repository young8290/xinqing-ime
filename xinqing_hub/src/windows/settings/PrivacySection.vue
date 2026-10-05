<script setup lang="ts">
// 设置中心 · 隐私（07 FR-SET-09 上半部分，放在“隐私与关于”分类里）：
// 采集内容说明（与引导页同一张表）、逐项同意与撤回（FR-DAT-05）、最近 20 次出网记录（只有字段名和时间）、导出我的数据（FR-DAT-03）。
// 删除全部数据（FR-DAT-04）、导入（FR-DAT-07）、隐私说明全文等 E 的命令和文本到位后加在这里。
import { onMounted, ref } from 'vue'
import { save } from '@tauri-apps/plugin-dialog'
import { commands, unwrap, type ConsentItem, type ConsentState, type NetLogView } from '@/api'
import { errorText, isCopyKey, t } from '@/i18n'
import { CONSENT_NUMBER, sendsToThirdParty } from '../shared/consent'
import NoticeTable from '../shared/NoticeTable.vue'

const consent = ref<ConsentState | null>(null)
const consentBusy = ref(false)
/** 刚撤回了会发给第三方的同意项：如实说明已发出的收不回来 */
const withdrawNotice = ref(false)
/** 撤回 ① 先确认一次：它是使用心晴的前提 */
const confirmSense = ref(false)
const consentError = ref<string | null>(null)

const netlog = ref<NetLogView[] | null>(null)
const netlogError = ref<string | null>(null)

const exporting = ref(false)
const exportDone = ref<string | null>(null)
const exportError = ref<string | null>(null)

onMounted(() => {
  void loadConsent()
  void loadNetlog()
})

async function loadConsent(): Promise<void> {
  try {
    consent.value = await unwrap(commands.consentGet())
  } catch (e) {
    consentError.value = errorText(e)
  }
}

async function setConsent(item: ConsentItem, granted: boolean): Promise<void> {
  consentError.value = null
  consentBusy.value = true
  try {
    consent.value = await unwrap(commands.consentSet(item, granted))
    withdrawNotice.value = !granted && sendsToThirdParty(item)
  } catch (e) {
    consentError.value = errorText(e)
  } finally {
    consentBusy.value = false
    confirmSense.value = false
  }
}

function onToggle(item: ConsentItem, e: Event): void {
  const box = e.target as HTMLInputElement
  if (item === 'sense' && !box.checked) {
    // 先不改，等用户在下面确认
    box.checked = true
    confirmSense.value = true
    return
  }
  void setConsent(item, box.checked)
}

async function loadNetlog(): Promise<void> {
  netlogError.value = null
  try {
    netlog.value = await unwrap(commands.aiNetLogRecent())
  } catch (e) {
    netlogError.value = errorText(e)
  }
}

/** `jev`、`llm/chat` → 中文用途；不认识的显示原值 */
function apiName(api: string): string {
  const key = `privacy.api.${api.replace('/', '_')}`
  return isCopyKey(key) ? t(key) : api
}

function result(r: NetLogView): string {
  if (r.status === 'ok') {
    return r.latency_ms === null
      ? t('privacy.netlog_ok_no_latency')
      : t('privacy.netlog_ok', { ms: r.latency_ms })
  }
  const key = `privacy.status.${r.status}`
  return t('privacy.netlog_fail', { reason: isCopyKey(key) ? t(key) : r.status })
}

const time = (ts: number) =>
  new Date(ts).toLocaleString('zh-CN', {
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })

function defaultFileName(): string {
  const d = new Date()
  const ymd = `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}`
  return `${t('privacy.export_file')}-${ymd}.zip`
}

async function exportData(): Promise<void> {
  exportDone.value = null
  exportError.value = null
  let path: string | null
  try {
    path = await save({
      defaultPath: defaultFileName(),
      filters: [{ name: t('privacy.export_filter'), extensions: ['zip'] }],
    })
  } catch (e) {
    exportError.value = errorText(e)
    return
  }
  // 用户关掉了对话框
  if (!path) return
  exporting.value = true
  try {
    await unwrap(commands.dataExport(path))
    exportDone.value = t('error.export_done', { path })
  } catch (e) {
    exportError.value = errorText(e)
  } finally {
    exporting.value = false
  }
}
</script>

<template>
  <h2>{{ t('privacy.title') }}</h2>

  <h3>{{ t('privacy.what') }}</h3>
  <NoticeTable />
  <p class="small">{{ t('onboarding.third_party') }}</p>

  <h3>{{ t('privacy.consent') }}</h3>
  <p class="muted small">{{ t('privacy.consent_hint') }}</p>
  <div v-if="consent" class="consents">
    <label v-for="e in consent.items" :key="e.item" class="consent" :data-item="e.item">
      <input
        type="checkbox"
        :checked="e.granted"
        :disabled="consentBusy"
        @change="onToggle(e.item, $event)"
      />
      <span>{{ CONSENT_NUMBER[e.item] }} {{ t(`consent.${e.item}`) }}</span>
    </label>
  </div>
  <div v-if="confirmSense" class="confirm" role="alertdialog" :aria-label="t('privacy.sense_confirm')">
    <p>{{ t('privacy.sense_confirm') }}</p>
    <button class="compact danger" :disabled="consentBusy" @click="setConsent('sense', false)">
      {{ t('privacy.sense_confirm_yes') }}
    </button>
    <button class="compact" @click="confirmSense = false">{{ t('privacy.sense_confirm_no') }}</button>
  </div>
  <p v-if="withdrawNotice" class="note" role="status">{{ t('error.withdraw_notice') }}</p>
  <p v-if="consentError" class="error" role="alert">{{ consentError }}</p>

  <h3>{{ t('privacy.netlog') }}</h3>
  <p class="muted small">{{ t('privacy.netlog_hint') }}</p>
  <p v-if="netlog && netlog.length === 0" class="muted">{{ t('privacy.netlog_empty') }}</p>
  <div v-else-if="netlog" class="table-wrap">
    <table class="netlog">
      <thead>
        <tr>
          <th scope="col">{{ t('privacy.netlog_col_time') }}</th>
          <th scope="col">{{ t('privacy.netlog_col_api') }}</th>
          <th scope="col">{{ t('privacy.netlog_col_fields') }}</th>
          <th scope="col">{{ t('privacy.netlog_col_result') }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(r, i) in netlog" :key="`${r.ts}-${i}`">
          <td class="nowrap">{{ time(r.ts) }}</td>
          <td>
            {{ apiName(r.api) }}
            <span v-if="r.model" class="model">{{ r.model }}</span>
          </td>
          <td class="fields">{{ r.fields.length ? r.fields.join('、') : t('privacy.netlog_no_fields') }}</td>
          <td :class="{ fail: r.status !== 'ok' }">{{ result(r) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
  <p v-if="netlogError" class="error" role="alert">{{ netlogError }}</p>
  <button class="compact" @click="loadNetlog()">{{ t('privacy.netlog_refresh') }}</button>

  <h3>{{ t('privacy.export') }}</h3>
  <p class="muted small">{{ t('privacy.export_hint') }}</p>
  <div class="row">
    <button :disabled="exporting" @click="exportData">
      {{ exporting ? t('privacy.exporting') : t('privacy.export_btn') }}
    </button>
    <span v-if="exportDone" class="done" role="status">{{ exportDone }}</span>
  </div>
  <p v-if="exportError" class="error" role="alert">{{ exportError }}</p>

  <p class="muted small pending">{{ t('privacy.pending') }}</p>
</template>

<style scoped>
h2 {
  margin-top: var(--xq-sp-5);
}

/* 小标题比 h2（16 px / 500）低一级：同字重、小一号、次要色 */
h3 {
  margin: var(--xq-sp-5) 0 var(--xq-sp-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-md);
  font-weight: 500;
}

.small {
  margin: 0 0 var(--xq-sp-2);
  font-size: var(--xq-fs-sm);
}

.consents {
  display: grid;
  gap: var(--xq-sp-1);
}

/* 整行都能点，点击目标不小于 32 px 高（DS-A11Y-03） */
.consent {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: flex-start;
  min-height: 32px;
  cursor: pointer;
}

.consent input {
  margin-top: 5px;
}

.confirm {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
  margin-top: var(--xq-sp-2);
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border: 1px solid var(--xq-danger);
  border-radius: var(--xq-radius-sm);
}

.confirm p {
  flex-basis: 100%;
  margin: 0;
}

.note {
  margin: var(--xq-sp-2) 0 0;
  padding: var(--xq-sp-2) var(--xq-sp-3);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  font-size: var(--xq-fs-sm);
}

.table-wrap {
  margin-bottom: var(--xq-sp-2);
  overflow-x: auto;
}

.netlog {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.netlog th,
.netlog td {
  padding: var(--xq-sp-1) var(--xq-sp-2);
  border-bottom: 1px solid var(--xq-border);
  text-align: left;
  vertical-align: top;
}

.nowrap {
  white-space: nowrap;
}

.model {
  display: block;
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
  overflow-wrap: anywhere;
}

.fields {
  overflow-wrap: anywhere;
}

.fail {
  color: var(--xq-danger);
}

.row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-3);
  align-items: center;
}

.done {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
  overflow-wrap: anywhere;
}

.compact {
  padding: 0 var(--xq-sp-3);
}

.danger {
  border-color: var(--xq-danger);
  color: var(--xq-danger);
}

.error {
  color: var(--xq-danger);
}

.pending {
  margin-top: var(--xq-sp-5);
}
</style>
