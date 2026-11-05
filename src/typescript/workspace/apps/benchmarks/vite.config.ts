import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

const baselinePath = resolve(
  fileURLToPath(new URL('.', import.meta.url)),
  '../../../../../research/BENCHMARK_BASELINE.md',
);

function benchmarkBaseline(): Plugin {
  return {
    name: 'benchmark-baseline',
    configureServer(server) {
      // The report lives outside this Vite app, so watch it explicitly and
      // refresh the page as soon as a local benchmark run updates it.
      server.watcher.add(baselinePath);
      server.watcher.on('change', (changedPath) => {
        if (changedPath === baselinePath) {
          server.ws.send({ type: 'full-reload' });
        }
      });
      server.middlewares.use(async (request, response, next) => {
        const requestedPath = request.url?.split('?')[0];
        if (
          requestedPath !== `${server.config.base}benchmark-baseline.md` &&
          requestedPath !== '/benchmark-baseline.md'
        ) {
          next();
          return;
        }
        try {
          response.setHeader('Content-Type', 'text/markdown; charset=utf-8');
          response.end(await readFile(baselinePath, 'utf8'));
        } catch {
          response.statusCode = 404;
          response.end('Benchmark baseline was not found.');
        }
      });
    },
    async generateBundle() {
      this.emitFile({
        type: 'asset',
        fileName: 'benchmark-baseline.md',
        source: await readFile(baselinePath, 'utf8'),
      });
    },
  };
}

export default defineConfig({
  base: '/benchmarks/',
  plugins: [react(), benchmarkBaseline()],
  server: {
    port: 5175,
    strictPort: true,
    watch: {
      // Source files live on the WSL-mounted workspace; polling catches edits
      // made by Windows editors when filesystem events do not cross the mount.
      usePolling: true,
    },
  },
});
