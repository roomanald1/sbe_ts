import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { nodePolyfills } from 'vite-plugin-node-polyfills';

const serviceUrl = (name: string, fallback: string) => process.env[`SBE_${name}_URL`] ?? fallback;
const positionManagementUrl = serviceUrl('POSITION_MANAGEMENT', 'http://localhost:3002');
const feedHandlersUrl = serviceUrl('FEED_HANDLERS', 'http://localhost:3001');
const executionUrl = serviceUrl('EXECUTION', 'http://localhost:3003');
const autoTraderUrl = serviceUrl('AUTO_TRADER', 'http://localhost:3005');
const insightsUrl = serviceUrl('INSIGHTS', 'http://localhost:3004');
const scoutsUrl = serviceUrl('SCOUTS', 'http://localhost:3011');

const apiProxy = {
  '/users': { target: positionManagementUrl, changeOrigin: true },
  '/instruments': { target: feedHandlersUrl, changeOrigin: true },
  '/positions/orders': { target: executionUrl, changeOrigin: true },
  '/positions/vwap': { target: executionUrl, changeOrigin: true },
  '/positions/close-profitable': { target: executionUrl, changeOrigin: true },
  '/positions/stream': { target: positionManagementUrl, changeOrigin: true, ws: true },
  '/positions/auto-trader/stream': { target: autoTraderUrl, changeOrigin: true, ws: true },
  '/positions/auto-trader': { target: autoTraderUrl, changeOrigin: true },
  '/positions/insights': { target: insightsUrl, changeOrigin: true },
  '/signals/stream': { target: insightsUrl, changeOrigin: true, ws: true },
  '/signals': { target: insightsUrl, changeOrigin: true },
  '/scouts': { target: scoutsUrl, changeOrigin: true },
  '/positions': { target: positionManagementUrl, changeOrigin: true },
  '/account': { target: positionManagementUrl, changeOrigin: true },
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
