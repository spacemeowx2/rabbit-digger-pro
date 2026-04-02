import { defineConfig } from 'vite-plus'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  fmt: {},
  lint: { options: { typeAware: true, typeCheck: true } },
  plugins: [react(), tailwindcss()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    proxy: {
      '/api': {
        target: process.env.RDP_API_BASE ?? 'http://127.0.0.1:9091',
        changeOrigin: true,
        ws: true,
      },
    },
  },
})
