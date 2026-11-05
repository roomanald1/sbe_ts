import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    watch: {
      // Source files live on the WSL-mounted workspace; polling catches edits
      // made by Windows editors when filesystem events do not cross the mount.
      usePolling: true,
    },
    proxy: {
      '/api': { target: 'http://localhost:8080', changeOrigin: true },
      '/benchmarks': {
        target: 'http://localhost:5175',
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
