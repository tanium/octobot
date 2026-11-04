import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

// In dev, the Vite server serves the UI and forwards API calls to a running
// octobot. Override the target with OCTOBOT_URL.
const octobot = process.env.OCTOBOT_URL ?? 'http://localhost:3000';

export default defineConfig({
  plugins: [svelte()],
  // No index.html fallback for unknown paths. Like octobot, dev serves only
  // real files, so pages are reached by hash (`#/users`).
  appType: 'mpa',
  // Tests run in jsdom, which needs Svelte's client build.
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  server: {
    proxy: {
      '/api': octobot,
      '/auth': octobot,
    },
  },
  test: {
    environment: 'jsdom',
  },
});
