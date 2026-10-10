<script setup lang="ts">
// 看板 · 日程与待办（06 FR-SCH-09、FR-SCH-13，07 FR-DSH-05）：
// - 待确认：识别出来、卡片收起后留下的日程与待办，可以添加 / 修改 / 忽略（FR-SCH-05 第 3 条）；
// - 已添加的日程：即将的按时间排，可以修改、删除、加入我的日历（单个或全部，FR-SCH-08）；已过期的收在下面；
// - 待办：未完成的可以完成、修改、删除，可以新建；已完成的收在下面（7 天后归档、30 天后删除由后端做）；
// - 已忽略的只显示数量（只留了去重的记号）。手动新建的不带 `AI 识别`。
// 从卡片层“修改”或底栏点进来时，`edit` 指明要打开哪一条的编辑框。
import { computed, onMounted, ref, watch } from 'vue'
import {
  commands,
  unwrap,
  type ConflictItem,
  type ScheduleInput,
  type ScheduleItem,
  type TodoInput,
  type TodoItem,
} from '@/api'
import { localDate } from '@/format'
import { errorText, t } from '@/i18n'
import { conflictText, flagTexts, scheduleWhen, todoDue } from '../cards/cards'
import ScheduleEditor from './ScheduleEditor.vue'
import TodoEditor from './TodoEditor.vue'
import { emptySchedule, parseEdit, scheduleForm, splitAdded, todoForm } from './agenda'

const props = defineProps<{ edit?: string }>()

const pendingS = ref<ScheduleItem[]>([])
const pendingT = ref<TodoItem[]>([])
const added = ref<ScheduleItem[]>([])
const ignored = ref(0)
const open = ref<TodoItem[]>([])
const done = ref<TodoItem[]>([])
const error = ref<string | null>(null)
const notice = ref<string | null>(null)
const busy = ref(false)
/** 正在编辑的：`new-schedule`、`new-todo`、`s:<id>`、`t:<id>` */
const editing = ref<string | null>(null)
/** 等确认删除的：`s:<id>` / `t:<id>` */
const deleting = ref<string | null>(null)

const split = computed(() => splitAdded(added.value, new Date()))

async function load(): Promise<void> {
  try {
    const [ps, pt, a, ig, o, d] = await Promise.all([
      unwrap(commands.scheduleList('pending')),
      unwrap(commands.todoList('pending')),
      unwrap(commands.scheduleList('added')),
      unwrap(commands.scheduleList('ignored')),
      unwrap(commands.todoList('open')),
      unwrap(commands.todoList('done')),
    ])
    pendingS.value = ps
    pendingT.value = pt
    added.value = a
    ignored.value = ig.length
    open.value = o
    done.value = d
    error.value = null
  } catch (e) {
    error.value = errorText(e)
  }
}

async function run(action: () => Promise<unknown>, after?: string): Promise<void> {
  busy.value = true
  notice.value = null
  error.value = null
  try {
    await action()
    if (after) notice.value = after
    await load()
  } catch (e) {
    error.value = errorText(e)
  } finally {
    busy.value = false
  }
}

function savedText(conflicts: ConflictItem[]): string {
  return conflicts.length
    ? t('dashboard.schedule.saved_conflict', { conflicts: conflicts.map(conflictText).join('；') })
    : t('dashboard.schedule.saved')
}

async function saveSchedule(id: number | null, input: ScheduleInput): Promise<void> {
  busy.value = true
  error.value = null
  try {
    const r = await unwrap(id === null ? commands.scheduleCreate(input) : commands.scheduleUpdate(id, input))
    notice.value = savedText(r.conflicts)
    editing.value = null
    await load()
  } catch (e) {
    error.value = errorText(e)
  } finally {
    busy.value = false
  }
}

async function saveTodo(id: number | null, input: TodoInput): Promise<void> {
  editing.value = null
  await run(
    () => unwrap(id === null ? commands.todoCreate(input) : commands.todoUpdate(id, input)),
    t('dashboard.schedule.saved'),
  )
}

async function exportIcs(ids: number[]): Promise<void> {
  await run(() => unwrap(commands.scheduleExportIcs(ids)), t('dashboard.schedule.exported'))
}

/** 列表里的图标（不是文案，只是记号） */
const ICON = { schedule: '📅', todo: '✅' } as const

/** 列表上的按钮：做完重新取一次 */
const act = {
  confirmSchedule: (id: number) => run(() => unwrap(commands.scheduleConfirm(id))),
  ignoreSchedule: (id: number) => run(() => unwrap(commands.scheduleIgnore(id))),
  deleteSchedule: (id: number) => ((deleting.value = null), run(() => unwrap(commands.scheduleDelete(id)))),
  confirmTodo: (id: number) => run(() => unwrap(commands.todoConfirm(id))),
  ignoreTodo: (id: number) => run(() => unwrap(commands.todoIgnore(id))),
  completeTodo: (id: number) => run(() => unwrap(commands.todoComplete(id))),
  deleteTodo: (id: number) => ((deleting.value = null), run(() => unwrap(commands.todoDelete(id)))),
}

const exportable = computed(() => split.value.upcoming.filter((s) => s.date !== null).map((s) => s.id))

function openEdit(edit: string | undefined): void {
  const target = parseEdit(edit)
  if (target) editing.value = `${target.kind === 'todo' ? 't' : 's'}:${target.id}`
}

onMounted(async () => {
  await load()
  openEdit(props.edit)
})
watch(() => props.edit, openEdit)
</script>

<template>
  <section>
    <div class="d-head">
      <h1>{{ t('dashboard.nav.schedule') }}</h1>
      <div class="d-row">
        <button @click="editing = 'new-schedule'">{{ t('dashboard.schedule.new_schedule') }}</button>
        <button @click="editing = 'new-todo'">{{ t('dashboard.schedule.new_todo') }}</button>
      </div>
    </div>
    <p v-if="error" class="d-error" role="alert">{{ error }}</p>
    <p v-if="notice" class="muted" role="status">{{ notice }}</p>

    <ScheduleEditor
      v-if="editing === 'new-schedule'"
      :initial="emptySchedule(localDate(new Date()))"
      :busy="busy"
      @save="(i) => saveSchedule(null, i)"
      @cancel="editing = null"
    />
    <TodoEditor
      v-if="editing === 'new-todo'"
      :initial="todoForm(null)"
      :busy="busy"
      @save="(i) => saveTodo(null, i)"
      @cancel="editing = null"
    />

    <section class="d-card block" aria-labelledby="pending-title">
      <h2 id="pending-title">{{ t('dashboard.schedule.pending') }}</h2>
      <p v-if="pendingS.length + pendingT.length === 0" class="muted">
        {{ t('dashboard.schedule.pending_empty') }}
      </p>
      <ul class="d-list">
        <li v-for="s in pendingS" :key="`s${s.id}`">
          <ScheduleEditor
            v-if="editing === `s:${s.id}`"
            :initial="scheduleForm(s)"
            :busy="busy"
            @save="(i) => saveSchedule(s.id, i)"
            @cancel="editing = null"
          />
          <div v-else class="item">
            <div class="what">
              <span class="title"
                ><span aria-hidden="true">{{ ICON.schedule }}</span> {{ s.title }}</span
              >
              <span v-if="s.source === 'ai'" class="d-tag">{{ t('common.ai_detected') }}</span>
              <span class="when">{{ scheduleWhen(s) }}</span>
              <span v-for="n in flagTexts(s.flags)" :key="n" class="note">{{ n }}</span>
            </div>
            <div class="d-row">
              <button class="primary" :disabled="busy" @click="act.confirmSchedule(s.id)">
                {{ t('dashboard.schedule.confirm') }}
              </button>
              <button @click="editing = `s:${s.id}`">{{ t('dashboard.schedule.edit') }}</button>
              <button :disabled="busy" @click="act.ignoreSchedule(s.id)">
                {{ t('dashboard.schedule.ignore') }}
              </button>
            </div>
          </div>
        </li>
        <li v-for="td in pendingT" :key="`t${td.id}`">
          <TodoEditor
            v-if="editing === `t:${td.id}`"
            :initial="todoForm(td)"
            :busy="busy"
            @save="(i) => saveTodo(td.id, i)"
            @cancel="editing = null"
          />
          <div v-else class="item">
            <div class="what">
              <span class="title"
                ><span aria-hidden="true">{{ ICON.todo }}</span> {{ td.title }}</span
              >
              <span v-if="td.source === 'ai'" class="d-tag">{{ t('common.ai_detected') }}</span>
              <span v-if="todoDue(td)" class="when">{{ todoDue(td) }}</span>
            </div>
            <div class="d-row">
              <button class="primary" :disabled="busy" @click="act.confirmTodo(td.id)">
                {{ t('dashboard.schedule.todo_confirm') }}
              </button>
              <button @click="editing = `t:${td.id}`">{{ t('dashboard.schedule.edit') }}</button>
              <button :disabled="busy" @click="act.ignoreTodo(td.id)">
                {{ t('dashboard.schedule.ignore') }}
              </button>
            </div>
          </div>
        </li>
      </ul>
    </section>

    <section class="d-card block" aria-labelledby="added-title">
      <div class="d-head">
        <h2 id="added-title">{{ t('dashboard.schedule.upcoming') }}</h2>
        <button v-if="exportable.length > 1" :disabled="busy" @click="exportIcs(exportable)">
          {{ t('dashboard.schedule.export_all') }}
        </button>
      </div>
      <p v-if="split.upcoming.length === 0" class="muted">{{ t('dashboard.schedule.upcoming_empty') }}</p>
      <ul class="d-list">
        <li v-for="s in split.upcoming" :key="s.id">
          <ScheduleEditor
            v-if="editing === `s:${s.id}`"
            :initial="scheduleForm(s)"
            :busy="busy"
            @save="(i) => saveSchedule(s.id, i)"
            @cancel="editing = null"
          />
          <div v-else class="item">
            <div class="what">
              <span class="title"
                ><span aria-hidden="true">{{ ICON.schedule }}</span> {{ s.title }}</span
              >
              <span v-if="s.source === 'ai'" class="d-tag">{{ t('common.ai_detected') }}</span>
              <span class="when">{{ scheduleWhen(s) }}</span>
              <span v-if="s.location" class="note">{{ t('cards.location', { location: s.location }) }}</span>
            </div>
            <div class="d-row">
              <button @click="editing = `s:${s.id}`">{{ t('dashboard.schedule.edit') }}</button>
              <button v-if="s.date" :disabled="busy" @click="exportIcs([s.id])">
                {{ t('dashboard.schedule.export') }}
              </button>
              <template v-if="deleting === `s:${s.id}`">
                <button class="danger" :disabled="busy" @click="act.deleteSchedule(s.id)">
                  {{ t('dashboard.schedule.delete_confirm') }}
                </button>
                <button @click="deleting = null">{{ t('dashboard.schedule.cancel') }}</button>
              </template>
              <button v-else @click="deleting = `s:${s.id}`">{{ t('dashboard.schedule.delete') }}</button>
            </div>
          </div>
        </li>
      </ul>
      <details v-if="split.expired.length">
        <summary>{{ t('dashboard.schedule.expired', { n: split.expired.length }) }}</summary>
        <ul class="d-list">
          <li v-for="s in split.expired" :key="s.id" class="item">
            <div class="what muted">
              <span class="title">{{ s.title }}</span>
              <span class="when">{{ scheduleWhen(s) }}</span>
            </div>
            <button :disabled="busy" @click="act.deleteSchedule(s.id)">
              {{ t('dashboard.schedule.delete') }}
            </button>
          </li>
        </ul>
      </details>
      <p v-if="ignored > 0" class="muted d-small">{{ t('dashboard.schedule.ignored', { n: ignored }) }}</p>
    </section>

    <section class="d-card block" aria-labelledby="todo-title">
      <h2 id="todo-title">{{ t('dashboard.schedule.todos') }}</h2>
      <p v-if="open.length === 0" class="muted">{{ t('dashboard.schedule.todos_empty') }}</p>
      <ul class="d-list">
        <li v-for="td in open" :key="td.id">
          <TodoEditor
            v-if="editing === `t:${td.id}`"
            :initial="todoForm(td)"
            :busy="busy"
            @save="(i) => saveTodo(td.id, i)"
            @cancel="editing = null"
          />
          <div v-else class="item">
            <div class="what">
              <button
                class="check"
                :disabled="busy"
                :aria-label="t('dashboard.schedule.complete_label', { title: td.title ?? '' })"
                :title="t('dashboard.schedule.complete')"
                @click="act.completeTodo(td.id)"
              />
              <span class="title">{{ td.title }}</span>
              <span v-if="td.source === 'ai'" class="d-tag">{{ t('common.ai_detected') }}</span>
              <span v-if="todoDue(td)" class="when">{{ todoDue(td) }}</span>
            </div>
            <div class="d-row">
              <button @click="editing = `t:${td.id}`">{{ t('dashboard.schedule.edit') }}</button>
              <template v-if="deleting === `t:${td.id}`">
                <button class="danger" :disabled="busy" @click="act.deleteTodo(td.id)">
                  {{ t('dashboard.schedule.delete_confirm') }}
                </button>
                <button @click="deleting = null">{{ t('dashboard.schedule.cancel') }}</button>
              </template>
              <button v-else @click="deleting = `t:${td.id}`">{{ t('dashboard.schedule.delete') }}</button>
            </div>
          </div>
        </li>
      </ul>
      <details v-if="done.length">
        <summary>{{ t('dashboard.schedule.done', { n: done.length }) }}</summary>
        <ul class="d-list">
          <li v-for="td in done" :key="td.id" class="item muted">
            <span class="title done-title">{{ td.title }}</span>
          </li>
        </ul>
      </details>
    </section>
  </section>
</template>

<style scoped>
.block {
  margin-top: var(--xq-sp-4);
}

.item {
  display: flex;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
  justify-content: space-between;
}

.what {
  display: flex;
  flex: 1;
  flex-wrap: wrap;
  gap: var(--xq-sp-2);
  align-items: center;
  min-width: 200px;
}

.title {
  font-weight: 500;
}

.done-title {
  text-decoration: line-through;
}

.when,
.note {
  color: var(--xq-text-2);
  font-size: var(--xq-fs-sm);
}

/* 勾选完成：一个空心方框按钮，守 32 × 32 点击目标 */
.check {
  position: relative;
  padding: 0;
  border-color: transparent;
  background: transparent;
}

.check::before {
  content: '';
  position: absolute;
  inset: 8px;
  border: 1.5px solid var(--xq-text-2);
  border-radius: 4px;
}

.danger {
  border-color: var(--xq-danger);
  color: var(--xq-danger);
}

.muted {
  color: var(--xq-text-2);
}
</style>
