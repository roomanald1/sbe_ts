import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { nodePolyfills } from 'vite-plugin-node-polyfills';

const apiProxy = {
  '/users': { target: 'http://localhost:3002', changeOrigin: true },
  '/instruments': { target: 'http://localhost:3001', changeOrigin: true },
  '/positions/orders': { target: 'http://localhost:3003', changeOrigin: true },
  '/positions/vwap': { target: 'http://localhost:3003', changeOrigin: true },
  '/positions/close-profitable': { target: 'http://localhost:3003', changeOrigin: true },
  '/positions/auto-trader': { target: 'http://localhost:3005', changeOrigin: true },
  '/positions/insights': { target: 'http://localhost:3004', changeOrigin: true },
  '/signals': { target: 'http://localhost:3004', changeOrigin: true },
  '/positions': { target: 'http://localhost:3002', changeOrigin: true },
  '/account': { target: 'http://localhost:3002', changeOrigin: true },
};

export default defineConfig({
  plugins: [
    react(),
    nodePolyfills({ globals: { Buffer: true } }),
  ],
  define: { global: {} },
  server: { proxy: apiProxy },
  preview: { proxy: apiProxy },
});
