import { resolve as pathResolve } from 'node:path';
import { existsSync, statSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const EXTENSIONS = ['.ts', '.tsx', '.js', '.jsx', '/index.ts', '/index.tsx', '/index.js'];

export async function resolve(specifier, context, nextResolve) {
  try {
    return await nextResolve(specifier, context);
  } catch (err) {
    if (err.code === 'ERR_MODULE_NOT_FOUND' && specifier.startsWith('.')) {
      const parentDir = context.parentURL ? new URL('.', context.parentURL).pathname : process.cwd();
      for (const ext of EXTENSIONS) {
        const candidate = pathResolve(parentDir, specifier + ext);
        if (existsSync(candidate) && !statSync(candidate).isDirectory()) {
          return {
            url: pathToFileURL(candidate).href,
            shortCircuit: true,
          };
        }
      }
    }
    throw err;
  }
}

