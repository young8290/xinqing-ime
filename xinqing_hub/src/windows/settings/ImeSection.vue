<script setup lang="ts">
// 设置中心 · 输入法（07 FR-SET-02、ADR 0016）：经 ime_schema / ime_config_get / ime_config_set 读写清风核心的配置。
// 常用项有中文名称（imeForm.ts 的清单），其余按 schema 生成、按键名第一段分组放进“高级”，打开哪组才渲染哪组。
// 修改即保存，核心跳过的项把原因显示在控件下方；需要重启才生效时显示横幅（FR-SET-01）。
// 跟着别处的改动刷新（ADR 0017）：收到 `ime_config:changed`、窗口重新获得焦点（核心自己的语言栏、菜单改配置不广播）、
// 保存之后都重新取一次整份配置；取的过程中再来信号，只在结束后补取一次。
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { CommandError, commands, events, unwrap, type ImeConfigChange, type ImeField } from '@/api'
import { errorText, t } from '@/i18n'
import ImeFieldControl from './ImeFieldControl.vue'
import { advancedGroups, commonFields, effectiveField, getPath } from './imeForm'

type Phase = 'loading' | 'ready' | 'unavailable' | 'error'

const phase = ref<Phase>('loading')
const loadError = ref('')
const fields = ref<ImeField[]>([])
const values = ref<unknown>({})
/** 被核心跳过的项及原因 */
const errors = reactive<Record<string, string>>({})
const needsRestart = ref(false)
const saved = ref(false)
/** 已经展开过的高级分组（展开时才渲染里面的控件） */
const opened = reactive(new Set<string>())
let savedTimer: ReturnType<typeof setTimeout> | undefined
/** 正在取配置；期间又来了刷新信号时 `refetchAgain` 置位，取完再取一次 */
let fetching = false
let refetchAgain = false
let unlisten: (() => void) | undefined
let disposed = false

const common = computed(() => commonFields(fields.value).map((f) => effectiveField(f, values.value)))
const groups = computed(() => advancedGroups(fields.value))

/** 取 schema 和配置。`quiet`（后台触发的重新加载）时不切到“加载中”，原来的页面留到结果出来，不闪一下。 */
async function load(quiet = false): Promise<void> {
  if (!quiet) phase.value = 'loading'
  try {
    const [schema, config] = await Promise.all([
      unwrap(commands.imeSchema()),
      unwrap(commands.imeConfigGet()),
    ])
    fields.value = schema
    values.value = config.values
    phase.value = 'ready'
  } catch (e) {
    if (e instanceof CommandError && e.ui.code === 'ime.unavailable') {
      phase.value = 'unavailable'
    } else {
      loadError.value = errorText(e)
      phase.value = 'error'
    }
  }
}

/** 只重新取配置（核心可能把值规范化，或在别处被改过）。并发的信号合并成至多再取一次。 */
async function refresh(): Promise<void> {
  if (fetching) {
    refetchAgain = true
    return
  }
  fetching = true
  try {
    do {
      refetchAgain = false
      values.value = (await unwrap(commands.imeConfigGet())).values
    } while (refetchAgain)
  } catch {
    // 刷新失败（例如核心刚退出）不打断页面：下次信号或下次获得焦点再取
  } finally {
    fetching = false
  }
}

/** 别处可能改了配置：页面正常时只重取配置；核心刚连上、或页面停在“没运行 / 出错”时整页重新加载。 */
function onExternalChange(change?: ImeConfigChange): void {
  if (change?.needs_restart) needsRestart.value = true
  if (phase.value === 'ready' && change?.reason !== 'connected') void refresh()
  else if (phase.value !== 'loading') void load(true)
}

function onFocus(): void {
  onExternalChange()
}

async function save(key: string, value: unknown): Promise<void> {
  try {
    const r = await unwrap(commands.imeConfigSet([{ key, value }]))
    const skipped = r.skipped.find((s) => s.key === key)
    if (skipped) errors[key] = skipped.reason
    else delete errors[key]
    if (r.needs_restart) needsRestart.value = true
    if (!skipped) {
      saved.value = true
      clearTimeout(savedTimer)
      savedTimer = setTimeout(() => (saved.value = false), 2000)
    }
  } catch (e) {
    errors[key] = errorText(e)
    return
  }
  // 核心可能把值规范化（取整、改写），以它为准
  await refresh()
}

function onToggle(prefix: string, e: Event): void {
  if ((e.target as HTMLDetailsElement).open) opened.add(prefix)
}

onMounted(async () => {
  window.addEventListener('focus', onFocus)
  void load()
  try {
    const off = await events.imeConfigChanged.listen((e) => onExternalChange(e.payload))
    // 订阅完成前组件已经卸载：立刻退订
    if (disposed) off()
    else unlisten = off
  } catch {
    // 浏览器预览里没有 Tauri 事件：只靠焦点和保存后的刷新
  }
})

onUnmounted(() => {
  disposed = true
  window.removeEventListener('focus', onFocus)
  unlisten?.()
  clearTimeout(savedTimer)
})
</script>

<template>
  <section>
    <h1>{{ t('settings.ime') }}</h1>

    <p v-if="phase === 'unavailable'" class="notice" role="status">
      {{ t('error.ime_unavailable') }}
      <button class="compact" @click="load()">{{ t('common.retry') }}</button>
    </p>
    <p v-else-if="phase === 'error'" class="error" role="alert">
      {{ loadError }}
      <button class="compact" @click="load()">{{ t('common.retry') }}</button>
    </p>

    <template v-else-if="phase === 'ready'">
      <p v-if="needsRestart" class="banner" role="status">{{ t('error.restart_banner') }}</p>

      <h2>{{ t('ime.common') }}</h2>
      <ImeFieldControl
        v-for="f in common"
        :key="f.key"
        :field="f"
        :value="getPath(values, f.key)"
        :error="errors[f.key]"
        @save="save(f.key, $event)"
      />

      <h2>{{ t('ime.advanced') }}</h2>
      <p class="muted">{{ t('ime.advanced_hint') }}</p>
      <details v-for="g in groups" :key="g.prefix" class="group" @toggle="onToggle(g.prefix, $event)">
        <summary>
          <span class="prefix">{{ g.prefix }}</span>
          <span class="count">{{ g.fields.length }}</span>
        </summary>
        <template v-if="opened.has(g.prefix)">
          <ImeFieldControl
            v-for="f in g.fields"
            :key="f.key"
            :field="f"
            :value="getPath(values, f.key)"
            :error="errors[f.key]"
            @save="save(f.key, $event)"
          />
        </template>
      </details>
    </template>

    <p v-if="saved" class="saved" role="status">{{ t('settings.saved') }}</p>
  </section>
</template>

<style scoped>
h2 {
  margin-top: var(--xq-sp-5);
}

.notice,
.banner {
  display: flex;
  gap: var(--xq-sp-3);
  align-items: center;
  padding: var(--xq-sp-3) var(--xq-sp-4);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
}

.error {
  color: var(--xq-danger);
}

.compact {
  padding: 0 var(--xq-sp-3);
}

.group {
  border-bottom: 1px solid var(--xq-border);
}

.group summary {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
  min-height: 40px;
  cursor: pointer;
}

.prefix {
  font-family: ui-monospace, Consolas, monospace;
}

.count {
  color: var(--xq-text-3);
  font-size: var(--xq-fs-xs);
}

/* 底部短暂提示“已保存”（FR-SET-01） */
.saved {
  position: fixed;
  bottom: var(--xq-sp-4);
  left: 50%;
  margin: 0;
  padding: var(--xq-sp-1) var(--xq-sp-4);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  box-shadow: var(--xq-shadow-float);
  transform: translateX(-50%);
}
</style>
