<script setup lang="ts">
// 情绪日记的编辑框（05 FR-DIA-01～03）：新写一篇时先请晴晴起草（diary_generate，最长约 20 秒；大模型不可用时给空白模板），
// 放进编辑框让用户改；改已有的就直接编辑。保存时后端按改没改过定来源，标“AI 生成”/“AI 辅助生成”/不标（FR-DIA-02）；
// 日记里命中危机词表时后端会打开对话窗口并显示求助卡片（FR-SAF-02 第 2 条）。
// 看板“写今天的日记”、对话窗口“写成情绪日记”（带那段对话的 session_id）共用。
import { computed, nextTick, onMounted, ref } from 'vue'
import { commands, unwrap, type DiaryItem, type DiarySaved } from '@/api'
import { errorText, t } from '@/i18n'

/** 一篇日记最多 5000 字（与后端 diary::MAX_CHARS 一致） */
const DIARY_MAX = 5000

const props = withDefaults(defineProps<{ sessionId?: number | null; editing?: DiaryItem | null }>(), {
  sessionId: null,
  editing: null,
})
const emit = defineEmits<{ saved: [result: DiarySaved]; cancel: [] }>()

const content = ref(props.editing?.content ?? '')
const draftId = ref<number | null>(null)
/** 晴晴起的草稿原文：没改过标“AI 生成”，改过标“AI 辅助生成” */
const draft = ref<string | null>(null)
const generating = ref(props.editing === null)
const saving = ref(false)
const error = ref<string | null>(null)
const area = ref<HTMLTextAreaElement | null>(null)

const tag = computed(() => {
  if (props.editing) {
    if (props.editing.source === 'ai_draft' && content.value === props.editing.content)
      return t('diary.ai_draft')
    return props.editing.source === 'manual' ? null : t('diary.ai_edited')
  }
  if (draft.value === null) return null
  return content.value === draft.value ? t('diary.ai_draft') : t('diary.ai_edited')
})

onMounted(async () => {
  if (!props.editing) {
    try {
      const d = await unwrap(commands.diaryGenerate(props.sessionId))
      content.value = d.text
      draftId.value = d.draft_id
      draft.value = d.ai_generated ? d.text : null
    } catch (e) {
      // 起草失败就让用户自己写，空白模板是后端的文案，这里取不到就留空
      error.value = errorText(e)
    } finally {
      generating.value = false
    }
  }
  await nextTick()
  area.value?.focus()
})

async function save(): Promise<void> {
  error.value = null
  const text = content.value.trim()
  if (!text) return void (error.value = t('diary.empty'))
  if ([...text].length > DIARY_MAX) return void (error.value = t('diary.too_long'))
  saving.value = true
  try {
    emit('saved', await unwrap(commands.diarySave(props.editing?.id ?? null, draftId.value, content.value)))
  } catch (e) {
    error.value = errorText(e)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <form
    class="diary-editor"
    :aria-label="t('diary.editor')"
    @submit.prevent="save"
    @keydown.esc.stop="emit('cancel')"
  >
    <div class="head">
      <h2>{{ t('diary.editor') }}</h2>
      <span v-if="tag" class="tag">{{ tag }}</span>
    </div>
    <p v-if="generating" class="hint" role="status">{{ t('diary.generating') }}</p>
    <template v-else>
      <p v-if="!props.editing" class="hint">
        {{ draft !== null ? t('diary.draft_hint') : t('diary.blank_hint') }}
      </p>
      <textarea
        ref="area"
        v-model="content"
        class="area"
        rows="10"
        :aria-label="t('diary.editor_label')"
        :maxlength="DIARY_MAX"
      />
    </template>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <div class="actions">
      <button type="submit" class="primary" :disabled="generating || saving">{{ t('diary.save') }}</button>
      <button type="button" @click="emit('cancel')">{{ t('diary.cancel') }}</button>
    </div>
  </form>
</template>

<style scoped>
.diary-editor {
  display: flex;
  flex-direction: column;
  gap: var(--xq-sp-2);
}

.head {
  display: flex;
  gap: var(--xq-sp-2);
  align-items: center;
}

.head h2 {
  margin: 0;
}

.hint {
  margin: 0;
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
  line-height: var(--xq-lh-sm);
}

.area {
  width: 100%;
  padding: var(--xq-sp-2);
  border: 1px solid var(--xq-border);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-1);
  font: inherit;
  resize: vertical;
}

/* DS-COPY-05 */
.tag {
  padding: 0 var(--xq-sp-2);
  border-radius: var(--xq-radius-sm);
  background: var(--xq-surface-2);
  color: var(--xq-text-2);
  font-size: var(--xq-fs-xs);
  line-height: var(--xq-lh-xs);
}

.error {
  margin: 0;
  color: var(--xq-danger);
}

.actions {
  display: flex;
  gap: var(--xq-sp-2);
}
</style>
