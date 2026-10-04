/// <reference types="vitest/config" />
import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 每个窗口一个入口（17 第 3.1 节）。名字与 tauri.conf.json 的窗口 label、src/windows/<label>/ 一致。
export const WINDOWS = ['widget', 'chat', 'dashboard', 'settings', 'onboarding'] as const

const src = fileURLToPath(new URL('./src', import.meta.url))

export default defineConfig({
  root: `${src}/windows`,
  publicDir: false,
  plugins: [vue()],
  resolve: { alias: { '@': src } },
  // Tauri 约定：固定端口，不清屏，好让 cargo 的输出留在终端里
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    outDir: fileURLToPath(new URL('./dist', import.meta.url)),
    emptyOutDir: true,
    target: 'es2022',
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
