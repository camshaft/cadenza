import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Base URL for GitHub Pages deployment at camshaft.github.io/cadenza
  base: '/cadenza/',
  // Needed for wasm-bindgen generated code
  build: {
    target: 'esnext',
  },
  optimizeDeps: {
    exclude: ['cadenza-web'],
  },
})
