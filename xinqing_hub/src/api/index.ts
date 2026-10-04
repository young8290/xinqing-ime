// 命令与事件的薄封装（17 第 3.1 节）：类型全部来自生成的 bindings.ts，禁止手写 invoke 字符串。
import type { UiError } from './bindings'

export { commands, events } from './bindings'
export type * from './bindings'

/** 命令返回的 `UiError`，抛出后由界面用 `errorText` 转成文案。 */
export class CommandError extends Error {
  readonly ui: UiError

  constructor(ui: UiError) {
    super(ui.code)
    this.name = 'CommandError'
    this.ui = ui
  }

  get message_key(): string {
    return this.ui.message_key
  }
}

type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: UiError }

/** 把 `{status, data | error}` 拆开：成功返回数据，失败抛 `CommandError`。 */
export async function unwrap<T>(result: Promise<CommandResult<T>>): Promise<T> {
  const r = await result
  if (r.status === 'error') throw new CommandError(r.error)
  return r.data
}
