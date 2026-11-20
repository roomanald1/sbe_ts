import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { nodePolyfills } from 'vite-plugin-node-polyfills';

export default defineConfig({
  plugins: [react(), nodePolyfills({ globals: { Buffer: true } })],
  define: { global: {} },
  server: {
    proxy: {
      '/__rsocket/feed': { target: 'ws://feed-handlers:10001', ws: true },
      '/__rsocket/insights-rpc': { target: 'ws://insights:10004', ws: true },
      '/__rsocket/insights-browser': {
        target: 'ws://insights:10011',
        ws: true,
      },
      '/__rsocket/position': {
        target: 'ws://position-management:10005',
        ws: true,
      },
      '/__rsocket/paper-exchange': {
        target: 'ws://paper-exchange:10006',
        ws: true,
      },
      '/__rsocket/auto-trader': { target: 'ws://auto-trader:10006', ws: true },
      '/__rsocket/learning': { target: 'ws://learning:10010', ws: true },
    },
  },
});
