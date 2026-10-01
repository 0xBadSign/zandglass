// Local design check: viewport screenshots while scrolling through a page.
// Usage (WSL, in web/): npm run build && node shoot-scroll.mjs <outdir> <page> [width] [height]
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';

const out = process.argv[2] ?? 'test-results/design';
const name = process.argv[3] ?? 'index';
const width = Number(process.argv[4] ?? 1440);
const height = Number(process.argv[5] ?? 900);
const server = spawn('npx', ['vite', 'preview', '--port', '4181', '--strictPort'], { stdio: 'ignore' });
await new Promise((r) => setTimeout(r, 2500));
const browser = await chromium.launch();
try {
  const ctx = await browser.newContext({ viewport: { width, height }, colorScheme: 'dark' });
  const page = await ctx.newPage();
  await page.goto(`http://127.0.0.1:4181/${name === 'index' ? '' : name}`);
  await page.waitForTimeout(1500);
  const total = await page.evaluate(() => document.documentElement.scrollHeight);
  let i = 0;
  for (let y = 0; y < total; y += Math.round(height * 0.85)) {
    await page.evaluate((v) => window.scrollTo(0, v), y);
    await page.waitForTimeout(1300);
    await page.screenshot({ path: `${out}/${name}-${width}-s${String(i++).padStart(2, '0')}.png` });
  }
} finally {
  await browser.close();
  server.kill();
}
