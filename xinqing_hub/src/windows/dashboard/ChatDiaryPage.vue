<script setup lang="ts">
// 看板 · 对话与日记（07 FR-DSH-05，05 FR-DIA-03、FR-CHT-08）：
// - 情绪日记：新的在前，标“AI 生成”/“AI 辅助生成”（手写的不标，FR-DIA-02）；可以写今天的、修改、删除；
// - 和晴晴的对话：会话列表（标题、最后一句的日期），可以在对话窗口打开、删除；内容和搜索在对话窗口里看。
import { onMounted, ref } from 'vue'
import { commands, unwrap, type ChatSessionItem, type DiaryItem } from '@/api'
import { dayLabel, localDate } from '@/format'
import { errorText, t } from '@/i18n'
import DiaryEditor from '../shared/DiaryEditor.vue'
import { openChat } from '../shared/bus'

const diaries = ref<DiaryItem[] | null>(null)
const sessions = ref<ChatSessionItem[] | null>(null)
const error = ref<string | null>(null)
/** `new` 新写一篇，或正在修改的日记 */
const editing = ref<'new' | DiaryItem | null>(null)
/** 等确认删除的：`d:<id>` / `c:<id>` */
const deleting = ref<string | null>(null)

async function load(): Promise<void> {
  try {
    const [d, s] = await Promise.all([unwrap(commands.diaryList()), unwrap(commands.chatListSessions())])
    diaries.value = d
    sessions.value = s
    error.value = null
  } catch (e) {
    error.value = errorText(e)
  }
}

async function run(action: () => Promise<unknown>): Promise<void> {
  deleting.value = null
  try {
    await action()
    await load()
  } catch (e) {
    error.value = errorText(e)
  }
}

const removeDiary = (id: number) => run(() => unwrap(commands.diaryDelete(id)))
const removeChat = (id: number) => run(() => unwrap(commands.chatDelete(id)))

function sourceTag(d: DiaryItem): string | null {
  if (d.source === 'ai_draft') return t('diary.ai_draft')
  if (d.source === 'ai_edited') return t('diary.ai_edited')
  return null
}

function onSaved(): void {
  editing.value = null
  void load()
}

onMounted(load)
</script>

<template>
  <section>
    <h1>{{ t('dashboard.nav.chat_diary') }}</h1>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>

    <section class="d-card" aria-labelledby="diary-title">
      <div class="d-head">
        <h2 id="diary-title">{{ t('dashboard.chat_diary.diary') }}</h2>
        <button class="primary" :disabled="editing !== null" @click="editing = 'new'">
          {{ t('dashboard.chat_diary.write') }}
        </button>
      </div>
      <DiaryEditor v-if="editing === 'new'" @saved="onSaved" @cancel="editing = null" />
      <p v-if="diaries && diaries.length === 0 && editing !== 'new'" class="muted">
        {{ t('dashboard.chat_diary.diary_empty') }}
      </p>
      <ul v-else-if="diaries" class="d-list">
        <li v-for="d in diaries" :key="d.id">
          <DiaryEditor
            v-if="editing !== null && editing !== 'new' && editing.id === d.id"
            :editing="d"
            @saved="onSaved"
            @cancel="editing = null"
          />
          <article v-else>
            <div class="d-row">
              <h3 class="date">{{ dayLabel(d.date) }}</h3>
              <span v-if="sourceTag(d)" class="d-tag">{{ sourceTag(d) }}</span>
              <span class="spacer" />
              <button @click="editing = d">{{ t('dashboard.chat_diary.edit') }}</button>
              <template v-if="deleting === `d:${d.id}`">
                <span class="d-small">{{ t('dashboard.chat_diary.delete_confirm') }}</span>
                <button class="danger" @click="removeDiary(d.id)">
                  {{ t('dashboard.chat_diary.confirm') }}
                </button>
                <button @click="deleting = null">{{ t('dashboard.chat_diary.cancel') }}</button>
              </template>
              <button v-else @click="deleting = `d:${d.id}`">{{ t('dashboard.chat_diary.delete') }}</button>
            </div>
            <p class="content">{{ d.content }}</p>
          </article>
        </li>
      </ul>
    </section>

    <section class="d-card chats" aria-labelledby="chats-title">
      <h2 id="chats-title">{{ t('dashboard.chat_diary.chats') }}</h2>
      <p class="muted d-small">{{ t('dashboard.chat_diary.chats_hint') }}</p>
      <p v-if="sessions && sessions.length === 0" class="muted">{{ t('chat.history_empty') }}</p>
      <ul v-else-if="sessions" class="d-list">
        <li v-for="s in sessions" :key="s.id" class="d-row">
          <span class="session">{{ s.title }}</span>
          <span class="muted d-small">{{
            t('dashboard.chat_diary.last_at', { date: dayLabel(localDate(new Date(s.last_ts))) })
          }}</span>
          <button @click="run(() => openChat(s.id))">{{ t('dashboard.chat_diary.open') }}</button>
          <template v-if="deleting === `c:${s.id}`">
            <button class="danger" @click="removeChat(s.id)">
              {{ t('dashboard.chat_diary.confirm') }}
            </button>
            <button @click="deleting = null">{{ t('dashboard.chat_diary.cancel') }}</button>
          </template>
          <button v-else @click="deleting = `c:${s.id}`">{{ t('dashboard.chat_diary.delete_chat') }}</button>
        </li>
      </ul>
    </section>
  </section>
</template>

<style scoped>
.date {
  margin: 0;
  font-size: var(--xq-fs-md);
  font-weight: 500;
}

.spacer {
  flex: 1;
}

.content {
  margin: var(--xq-sp-2) 0 0;
  white-space: pre-wrap;
}

.chats {
  margin-top: var(--xq-sp-4);
}

.session {
  flex: 1;
  min-width: 120px;
}

.danger {
  border-color: var(--xq-danger);
  color: var(--xq-danger);
}

.muted {
  color: var(--xq-text-2);
}
</style>
