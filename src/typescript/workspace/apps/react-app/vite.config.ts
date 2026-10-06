import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { nodePolyfills } from 'vite-plugin-node-polyfills'

const apiProxy = {
  '/users': {
    target: 'http://localhost:3000',
    changeOrigin: true,
  },
}

export default defineConfig({
  plugins: [
    react(),
    nodePolyfills({
      globals: {
        Buffer: true,   // expose Buffer globally
      }
    })
  ],
  define: {
    global: {},        // required for some Node polyfills
  },
  server: {
    proxy: apiProxy,
  },
  preview: {
    proxy: apiProxy,
  },
})
