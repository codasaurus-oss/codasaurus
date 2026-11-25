import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    allowedHosts: ["codasaurus.thelohit.in"],
    proxy: {
      '/api': process.env.CODASAURUS_API ?? 'http://127.0.0.1:3000',
      '/health': process.env.CODASAURUS_API ?? 'http://127.0.0.1:3000',
    },
  },
})
