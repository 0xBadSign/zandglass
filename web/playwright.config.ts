import { defineConfig } from '@playwright/test';

// Tests run against the production build, served with the strict CSP from
// vite.config.ts. Run `npm run build` first (the `test:e2e` script does).
export default defineConfig({
  testDir: 'e2e',
  outputDir: 'test-results/artifacts',
  timeout: 60_000,
  fullyParallel: true,
  reporter: [['list']],
  use: { baseURL: 'http://127.0.0.1:4173', browserName: 'chromium' },
  projects: [
    { name: 'desktop', use: { viewport: { width: 1280, height: 860 } } },
    { name: 'phone', use: { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true } },
  ],
  webServer: {
    command: 'npx vite preview',
    url: 'http://127.0.0.1:4173/heir.html',
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
