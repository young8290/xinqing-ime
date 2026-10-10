<script setup lang="ts">
// 某天的暖心话（07 FR-DSH-02“今日一句”、FR-DSH-03 点开某天）：时间、句子、`AI 生成`、反馈状态；
// 今天的还没反馈过的可以在这里点 👍 / 👎 / 🔕（FR-CMF-05）。
import { onMounted, ref, watch } from 'vue'
import { commands, unwrap, type ComfortItem, type ComfortVerdict } from '@/api'
import { clockOf } from '@/format'
import { errorText, t } from '@/i18n'

const props = withDefaults(defineProps<{ date: string; canVote?: boolean }>(), { canVote: false })

const items = ref<ComfortItem[] | null>(null)
const error = ref<string | null>(null)
const VERDICTS: ComfortVerdict[] = ['useful', 'unfit', 'mute']

async function load(): Promise<void> {
  error.value = null
  try {
    items.value = await unwrap(commands.comfortList(props.date))
  } catch (e) {
    error.value = errorText(e)
  }
}

async function vote(item: ComfortItem, v: ComfortVerdict): Promise<void> {
  try {
    await unwrap(commands.comfortFeedback(item.id, v))
    item.feedback = v
  } catch (e) {
    error.value = errorText(e)
  }
}

onMounted(load)
watch(() => props.date, load)
</script>

<template>
  <div>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>
    <p v-else-if="items && items.length === 0" class="muted">{{ t('dashboard.today.comfort_empty') }}</p>
    <ul v-else-if="items" class="d-list">
      <li v-for="c in items" :key="c.id" class="comfort">
        <span class="d-time">{{ clockOf(c.ts) }}</span>
        <span class="text">{{ c.text }}</span>
        <span v-if="c.ai_generated" class="d-tag">{{ t('common.ai_generated') }}</span>
        <span class="feedback d-small">
          <template v-if="c.feedback">{{ t(`widget.comfort.${c.feedback}`) }}</template>
          <span v-else-if="props.canVote" class="d-row" role="group" :aria-label="t('widget.comfort.label')">
            <button
              v-for="v in VERDICTS"
              :key="v"
              class="vote"
              :data-verdict="v"
              :aria-label="t(`widget.comfort.${v}`)"
              :title="t(`widget.comfort.${v}`)"
              @click="vote(c, v)"
            >
              {{ [...t(`widget.comfort.${v}`)][0] }}
            </button>
          </span>
          <span v-else class="muted">{{ t('dashboard.today.feedback_none') }}</span>
        </span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.comfort {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
}

.text {
  flex: 1;
  min-width: 160px;
}

.feedback {
  color: var(--xq-text-2);
}

.vote {
  padding: 0 var(--xq-sp-1);
}

.muted {
  color: var(--xq-text-2);
}
</style>
