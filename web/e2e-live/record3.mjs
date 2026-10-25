// Demo-video recorder v3 (1920x1080): the clips added for the second video.
// Not part of the test suite. Writes raw clips and the time marks the editor uses.
//
//   OUT=../.private/video3/raw SEGMENTS=heir,claim,owner BASE=https://zandglass.cash \
//     KIT_FILE=<a locked, signed kit> OWNER_URL=http://127.0.0.1:8798/owner#t=... HEIR=age1... \
//     node e2e-live/record3.mjs
//
// Review the clips before publishing.
import { chromium } from '@playwright/test';
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';

const BASE = (process.env.BASE ?? 'https://zandglass.cash').replace(/\/$/, '');
const OUT = process.env.OUT ?? '../.private/video3/raw';
const SEGMENTS = (process.env.SEGMENTS ?? 'heir,claim,owner').split(',');
const SIZE = { width: 1920, height: 1080 };
mkdirSync(OUT, { recursive: true });
const manifestPath = `${OUT}/manifest.json`;
const manifest = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, 'utf8')) : {};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const browser = await chromium.launch();

async function clip(name, body) {
  const dir = `${OUT}/_tmp_${name}`;
  const context = await browser.newContext({
    viewport: SIZE,
    deviceScaleFactor: 1,
    colorScheme: 'dark',
    reducedMotion: 'no-preference',
    recordVideo: { dir, size: SIZE },
    acceptDownloads: true,
  });
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  const t0 = Date.now();
  const marks = {};
  const mark = (k) => {
    marks[k] = Number(((Date.now() - t0) / 1000).toFixed(2));
    console.log(`[${name} ${marks[k]}s] ${k}`);
  };
  try {
    await body(page, mark);
  } catch (e) {
    console.log(`[${name}] FAILED: ${e.message}`);
    process.exitCode = 1;
  }
  mark('end');
  await context.close();
  const f = readdirSync(dir).find((x) => x.endsWith('.webm'));
  if (f) renameSync(`${dir}/${f}`, `${OUT}/${name}.webm`);
  manifest[name] = marks;
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
}

/** Smooth scroll so the element's top sits `offset` px below the top, over `ms`. */
async function glide(page, selector, ms = 1400, offset = 80) {
  await page.evaluate(
    async ([sel, dur, off]) => {
      const el = document.querySelector(sel);
      if (!el) return;
      const from = window.scrollY;
      const to = Math.max(0, el.getBoundingClientRect().top + window.scrollY - off);
      const t0 = performance.now();
      await new Promise((done) => {
        const step = (t) => {
          const k = Math.min(1, (t - t0) / dur);
          const e = k < 0.5 ? 4 * k * k * k : 1 - Math.pow(-2 * k + 2, 3) / 2;
          window.scrollTo(0, from + (to - from) * e);
          if (k < 1) requestAnimationFrame(step);
          else done();
        };
        requestAnimationFrame(step);
      });
    },
    [selector, ms, offset],
  );
}

/** Heir page: make a key, confirm words, then the fire drill end to end. */
if (SEGMENTS.includes('heir')) {
  await clip('heir', async (page, mark) => {
    await page.goto(`${BASE}/heir`);
    await sleep(1500);
    mark('start');
    await page.getByRole('button', { name: 'Create my words' }).click();
    const words = await page.locator('.phrase li').allTextContents();
    await sleep(2500);
    mark('words');
    await page.getByRole('button', { name: 'I wrote them down' }).click();
    await sleep(800);
    const labels = await page.locator('label').allTextContents();
    for (const l of labels) {
      const p = Number(l.replace('Word ', ''));
      if (!Number.isFinite(p) || p < 1) continue;
      await page.getByLabel(`Word ${p}`, { exact: true }).pressSequentially(words[p - 1] ?? '', { delay: 30 });
    }
    await page.getByRole('button', { name: 'Check my words' }).click();
    await page.locator('#heir-code').waitFor();
    mark('code');
    await sleep(2500);
    const make = page.getByRole('button', { name: 'Make a practice kit' });
    await make.scrollIntoViewIfNeeded();
    await page.evaluate(() => window.scrollBy(0, 200));
    await sleep(1200);
    mark('drill');
    await make.click();
    await page.getByText('Your practice kit is ready').waitFor();
    await sleep(2200);
    mark('ready');
    await page.getByRole('button', { name: 'Open it on the Claim page' }).click();
    await page.waitForURL('**/claim');
    mark('claim');
    await sleep(2500);
    // The practice kit unlocks about a minute after it was made: wait for it.
    await page.getByRole('heading', { name: 'This kit is unlocked' }).waitFor({ timeout: 150_000 });
    mark('unlocked');
    await sleep(1000);
    await page.getByLabel('Your 24 words').pressSequentially(words.join(' '), { delay: 3 });
    await sleep(500);
    await page.getByRole('button', { name: 'Open the kit' }).click();
    await page.getByRole('heading', { name: 'Practice complete' }).waitFor({ timeout: 120_000 });
    mark('practice-done');
    await glide(page, 'h2, h3', 900, 160);
    await sleep(4000);
  });
}

/** Claim page with a locked, signed kit: owner pulse, signature, calendar. */
if (SEGMENTS.includes('claim')) {
  const KIT = process.env.KIT_FILE;
  if (!KIT) throw new Error('set KIT_FILE');
  await clip('claim', async (page, mark) => {
    await page.goto(`${BASE}/claim`);
    await sleep(1500);
    mark('start');
    await page.locator('#kit-file').setInputFiles({ name: 'plan-e2.zandglass', mimeType: 'application/json', buffer: readFileSync(KIT) });
    await page.locator('#owner-sig').filter({ hasText: 'Signed' }).waitFor();
    mark('loaded');
    await sleep(1500);
    await glide(page, '#owner-sig', 1200, 420);
    mark('pulse');
    await sleep(4500);
    await page.getByRole('button', { name: 'Add the unlock date to my calendar' }).click();
    await sleep(2500);
    mark('done');
  });
}

/** Owner panel (testnet, throwaway store): savings lock and paper kit. */
if (SEGMENTS.includes('owner')) {
  const URL = process.env.OWNER_URL;
  const HEIR = process.env.HEIR;
  if (!URL || !HEIR) throw new Error('set OWNER_URL and HEIR');
  await clip('owner', async (page, mark) => {
    await page.goto(URL);
    await page.getByRole('heading', { name: /Create your store|Unlock your store/ }).waitFor();
    mark('store');
    await sleep(1200);
    await page.getByRole('button', { name: 'Testnet (test coins)' }).click().catch(() => undefined);
    await sleep(600);
    const pass = 'video-demo-passphrase';
    await page.locator('#pass1').pressSequentially(pass, { delay: 20 });
    if (await page.locator('#pass2').isVisible()) {
      await page.locator('#pass2').pressSequentially(pass, { delay: 10 });
      await page.getByRole('button', { name: 'Create the store' }).click();
    } else {
      await page.getByRole('button', { name: 'Unlock' }).click();
    }
    await page.getByRole('heading', { name: 'New plan', exact: true }).waitFor({ timeout: 120_000 });
    mark('newplan');
    await sleep(1200);
    // An inheritance plan, for the paper kit.
    await page.locator('#code0').pressSequentially(HEIR, { delay: 6 });
    await page.selectOption('#every', '90d');
    await page.locator('#amount').pressSequentially('0.05', { delay: 50 });
    await page.locator('#message').pressSequentially('For Deniz. Look after each other.', { delay: 20 });
    await sleep(600);
    await page.getByRole('button', { name: 'Create the plan' }).click();
    await page.locator('#created-title').waitFor({ timeout: 180_000 });
    mark('created');
    await sleep(3500);
    await page.getByRole('button', { name: 'Done' }).click();
    await glide(page, '.plan-card', 1200, 160);
    mark('plan');
    await sleep(1500);
    await page.getByRole('button', { name: 'Print paper kit' }).click();
    await sleep(500);
    await glide(page, '.paper-sheet, #paper-print, .paper', 1200, 120);
    mark('paper');
    await sleep(4500);
    // A savings lock (testnet, throwaway store): created for real, seed deleted.
    await glide(page, '.app-flow', 900, 40);
    await page.getByRole('button', { name: 'New plan' }).click();
    await page.getByRole('button', { name: /Savings lock/ }).click();
    mark('lock');
    await sleep(1000);
    await page.locator('#lock-heir').pressSequentially(HEIR, { delay: 6 });
    await page.selectOption('#lock-until', '365d').catch(() => undefined);
    await page.locator('#lock-message').pressSequentially('Do not touch before next year.', { delay: 20 });
    await glide(page, '#lock-understood', 900, 300);
    await page.locator('#lock-understood').check();
    await page.locator('#lock-confirm').pressSequentially('LOCK', { delay: 120 });
    await sleep(800);
    mark('lock-submit');
    await page.getByRole('button', { name: 'Create the savings lock' }).click();
    await page.locator('.lock-line').first().waitFor({ timeout: 180_000 });
    const done = page.getByRole('button', { name: 'Done' });
    if (await done.isVisible().catch(() => false)) {
      await sleep(3000);
      await done.click();
    }
    await glide(page, '.lock-line', 1000, 260);
    mark('lock-card');
    await sleep(4500);
  });
}

await browser.close();
