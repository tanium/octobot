import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vite';

// In dev, the Vite server serves the UI and forwards API calls to a running
// octobot. Override the target with OCTOBOT_URL.
const octobot = process.env.OCTOBOT_URL ?? 'http://localhost:3000';

export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': octobot,
      '/auth': octobot,
    },
  },
});
