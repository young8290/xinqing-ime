import js from '@eslint/js'
import pluginVue from 'eslint-plugin-vue'
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript'
import prettier from 'eslint-config-prettier'
import globals from 'globals'

export default defineConfigWithVueTs(
  // 生成的文件只由生成器负责（bindings.ts 来自 export-bindings，zh-CN.ts 来自 gen-i18n）
  { ignores: ['dist/**', 'src-tauri/**', 'src/api/bindings.ts', 'src/i18n/zh-CN.ts'] },
  js.configs.recommended,
  pluginVue.configs['flat/recommended'],
  vueTsConfigs.recommended,
  {
    languageOptions: { globals: { ...globals.browser } },
    rules: {
      // 固定文案只能来自 hub_templates/ui_copy.toml（17 第 3.1 节 i18n 单一来源）
      'vue/no-bare-strings-in-template': ['error', { allowlist: ['·', '|', '%', '(', ')', '…'] }],
    },
  },
  {
    files: ['scripts/**', 'vite.config.ts', 'eslint.config.js'],
    languageOptions: { globals: { ...globals.node } },
  },
  prettier,
)
