// “关于”页的开源许可列表（07 FR-SET-09、02 C-LAW-08）。
// 清风的 MIT 许可与第三方资源声明直接从仓库根目录的 LICENSE、NOTICE.md 打包进来，原文随仓库更新，不另抄一份。
// 新增随发行版分发的组件、字体、动画素材或图标库（例如 Lucide，DS-ICON）时在这里加一条。
import windInputLicense from '../../../../LICENSE?raw'
import windInputNotice from '../../../../NOTICE.md?raw'
import { t } from '@/i18n'

export interface LicenseEntry {
  name: string
  /** SPDX 表达式，或说明文字 */
  license: string
  /** 有全文时可以展开查看 */
  text?: string
}

export function licenseEntries(): LicenseEntry[] {
  return [
    { name: 'WindInput（清风输入法）', license: 'MIT', text: windInputLicense },
    { name: t('about.notice_title'), license: t('about.notice_license'), text: windInputNotice },
    { name: 'Tauri', license: 'Apache-2.0 OR MIT' },
    { name: 'Vue', license: 'MIT' },
    { name: 'Pinia', license: 'MIT' },
  ]
}
