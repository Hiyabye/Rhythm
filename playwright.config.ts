import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/ui',
  fullyParallel: true,
  workers: 2,
  timeout: 20_000,
  expect: { timeout: 5_000 },
  reporter: 'list',
  outputDir: 'node_modules/.cache/playwright-results',
  use: { baseURL: 'http://127.0.0.1:4178', viewport: { width: 1180, height: 800 }, trace: 'retain-on-failure' },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }, { name: 'webkit', use: { browserName: 'webkit' } }],
  webServer: {
    command: 'npm run build && npm exec vite preview -- --host 127.0.0.1 --port 4178 --strictPort',
    url: 'http://127.0.0.1:4178',
    timeout: 120_000,
    reuseExistingServer: false,
  },
});
