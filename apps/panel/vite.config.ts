import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// El relay de Atlas sirve el build en el mismo host bajo /panel/, así que
// la base pública no puede ser la raíz.
export default defineConfig({
  base: '/panel/',
  plugins: [react()],
  server: { port: 5174 },
  build: { outDir: 'dist', chunkSizeWarningLimit: 900 },
})
