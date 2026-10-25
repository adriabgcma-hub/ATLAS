import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// El relay de Atlas sirve el build en el mismo host bajo /panel/, así que
// la base pública no puede ser la raíz. La app macOS (apps/panel-app) usa el
// mismo frontend con base relativa y su propio directorio de salida:
// `PANEL_BASE=./ PANEL_OUT_DIR=../panel-app/dist`.
const base = process.env.PANEL_BASE ?? '/panel/'
const outDir = process.env.PANEL_OUT_DIR ?? 'dist'

export default defineConfig({
  base,
  plugins: [react()],
  server: { port: 5174 },
  build: { outDir, emptyOutDir: true, chunkSizeWarningLimit: 900 },
})
