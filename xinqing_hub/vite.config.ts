/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 每个窗口一个入口（17 第 3.1 节）。名字与 tauri.conf.json 的窗口 label、src/windows/<label>/ 一致。
export const WINDOWS = ['widget', 'chat', 'dashboard', 'settings', 'onboarding', 'cards'] as const

const src = fileURLToPath(new URL('./src', import.meta.url))

export default defineConfig({
  root: `${src}/windows`,
  publicDir: false,
  plugins: [vue()],
  resolve: { alias: { '@': src } },
  // Tauri 约定：固定端口，不清屏，好让 cargo 的输出留在终端里
  clearScreen: false,
  // “关于”页把仓库根目录的 LICENSE、NOTICE.md 打包进来（settings/licenses.ts），开发服务器要允许读到仓库根目录
  server: { port: 1420, strictPort: true, fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
  build: {
    outDir: fileURLToPath(new URL('./dist', import.meta.url)),
    emptyOutDir: true,
    target: 'es2022',
    // 看板窗口带 ECharts（折线、条形、热力图），单包约 600 kB；都是安装目录里的本地文件，不走网络，只在看板窗口加载
    chunkSizeWarningLimit: 700,
    rollupOptions: {
      input: Object.fromEntries(WINDOWS.map((w) => [w, `${src}/windows/${w}/index.html`])),
    },
  },
  test: {
    root: fileURLToPath(new URL('.', import.meta.url)),
    environment: 'jsdom',
    include: ['src/**/*.test.ts', 'scripts/**/*.test.ts'],
  },
})
