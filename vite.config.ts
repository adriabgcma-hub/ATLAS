import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  build: {
    // Three.js es un vendor monolítico (~892 kB minificado); ya está aislado
    // del bundle inicial y no puede dividirse sin romper su runtime.
    chunkSizeWarningLimit: 1000,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/three/') || id.includes('/node_modules/@react-three/fiber/')) {
            return 'three'
          }
        },
      },
    },
  },
})
