// Local design check: screenshots of the built site (not part of the test suite).
// Usage (WSL, in web/): npm run build && node shoot.mjs <outdir> [page...]
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';

const out = process.argv[2] ?? 'test-results/design';
const pages = process.argv.slice(3).length ? process.argv.slice(3) : ['index'];
const server = spawn('npx', ['vite', 'preview', '--port', '4180', '--strictPort'], { stdio: 'ignore' });
await new Promise((r) => setTimeout(r, 2500));
const browser = await chromium.launch();
try {
  for (const [name, vp] of [
    ['desktop', { width: 1440, height: 900 }],
    ['phone', { width: 390, height: 844 }],
  ]) {
    const ctx = await browser.newContext({ viewport: vp, deviceScaleFactor: name === 'phone' ? 2 : 1, colorScheme: 'dark' });
    const page = await ctx.newPage();
    for (const p of pages) {
      await page.goto(`http://127.0.0.1:4180/${p === 'index' ? '' : p}`);
      await page.waitForTimeout(1600);
      await page.screenshot({ path: `${out}/${p}-${name}-fold.png` });
      await page.screenshot({ path: `${out}/${p}-${name}-full.png`, fullPage: true });
    }
    await ctx.close();
  }
} finally {
  await browser.close();
  server.kill();
}
