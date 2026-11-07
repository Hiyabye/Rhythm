import { defineConfig } from '@playwright/test';
import base from './playwright.config';

// README screenshots only (`npm run screenshots`): Chromium, its own port so it
// never collides with a running `npm run test:ui` server.
const port = 4179;
export default defineConfig({
  ...base,
  testMatch: 'screenshots.spec.ts',
  fullyParallel: false,
  workers: 1,
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
  use: { ...base.use, baseURL: `http://127.0.0.1:${port}` },
  webServer: {
    command: `npm run build && npm exec vite preview -- --host 127.0.0.1 --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    timeout: 120_000,
    reuseExistingServer: false,
  },
});
