// AI 服务表单（引导页 FR-ONB-05、设置页 FR-SET-08 共用，ADR 0012）：表单内容 ↔ `secrets_set` 的入参。
// 密钥只进不出：界面拿不到已保存的密钥，密钥框留空就传 null（沿用旧的），占位里显示末 4 位。
import type { AiConfigInput, AiConfigSource, AiConfigView } from '@/api'
import { t } from '@/i18n'

export interface AiForm {
  jevUrl: string
  jevKey: string
  llmUrl: string
  llmKey: string
  /** 大模型按优先级排列；`undefined` 表示表单不管模型（引导页），沿用已保存的 */
  models?: string[]
}

/** 地址为空就是不用这项服务（传 null）。Jev 的模型界面不改，沿用已保存的（没有就用默认）。 */
export function toAiInput(f: AiForm, view: AiConfigView | null): AiConfigInput {
  const jev = f.jevUrl.trim()
  const llm = f.llmUrl.trim()
  return {
    jev: jev ? { base_url: jev, api_key: f.jevKey.trim() || null, model: view?.jev?.model ?? null } : null,
    llm: llm
      ? {
          base_url: llm,
          api_key: f.llmKey.trim() || null,
          models: (f.models ?? view?.llm?.models ?? []).map((m) => m.trim()).filter((m) => m !== ''),
        }
      : null,
  }
}

/** 和已保存的相比改了什么没有（填了新密钥、改了地址或模型）。 */
export function aiFormDirty(f: AiForm, view: AiConfigView | null): boolean {
  const saved = view?.llm?.models ?? []
  return (
    f.jevKey.trim() !== '' ||
    f.llmKey.trim() !== '' ||
    f.jevUrl.trim() !== (view?.jev?.base_url ?? '') ||
    f.llmUrl.trim() !== (view?.llm?.base_url ?? '') ||
    (f.models !== undefined && f.models.join('\n') !== saved.join('\n'))
  )
}

/** 开发版连着 secrets.toml / mock-ai 时的一行提示；发行版没有。 */
export function devHint(source: AiConfigSource | undefined): string | null {
  if (source === 'dev_file') return t('ai_service.dev_file')
  if (source === 'dev_mock') return t('ai_service.dev_mock')
  return null
}

export function keyPlaceholder(tail: string | null | undefined): string {
  return tail ? t('ai_service.key_saved', { tail }) : ''
}
