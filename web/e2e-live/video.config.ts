import { defineConfig } from '@playwright/test';

// Demo-video clips recorded through the test runner (they reuse the test
// helpers). Run: npx playwright test -c e2e-live/video.config.ts
export default defineConfig({
  testDir: '.',
  testMatch: 'video-*.spec.ts',
  outputDir: '../../.private/video2/runner',
  timeout: 120_000,
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:4173',
    browserName: 'chromium',
    viewport: { width: 1920, height: 1080 },
    colorScheme: 'dark',
    video: { mode: 'on', size: { width: 1920, height: 1080 } },
  },
  webServer: {
    command: 'npx vite preview',
    cwd: '..',
    url: 'http://127.0.0.1:4173/heir.html',
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
