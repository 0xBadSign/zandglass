// Final demo-video recorder (1920x1080). Not part of the test suite.
// Records raw clips with Playwright and writes the time marks the editor uses.
//
//   OUT=../.private/video2/raw SEGMENTS=landing,playground,owner \
//   BASE=https://zandglass.cash JUDGE_CODE=... OWNER_URL=http://127.0.0.1:8799/owner#t=... HEIR=age1... \
//   node e2e-live/record2.mjs
//
// The judge code is typed into a password field (shown as dots). Review the
// clips before publishing.
import { chromium } from '@playwright/test';
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';

const BASE = (process.env.BASE ?? 'https://zandglass.cash').replace(/\/$/, '');
const OUT = process.env.OUT ?? '../.private/video2/raw';
const SEGMENTS = (process.env.SEGMENTS ?? 'landing,playground,owner').split(',');
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

/** Smooth scroll to an element's top (minus an offset) over `ms`. */
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

/** Smooth scroll by a number of pixels over `ms`. */
async function glideBy(page, dy, ms) {
  await page.evaluate(
    async ([d, dur]) => {
      const from = window.scrollY;
      const t0 = performance.now();
      await new Promise((done) => {
        const step = (t) => {
          const k = Math.min(1, (t - t0) / dur);
          window.scrollTo(0, from + d * (0.5 - Math.cos(Math.PI * k) / 2));
          if (k < 1) requestAnimationFrame(step);
          else done();
        };
        requestAnimationFrame(step);
      });
    },
    [dy, ms],
  );
}

if (SEGMENTS.includes('landing')) {
  await clip('landing', async (page, mark) => {
    await page.goto(`${BASE}/`);
    await sleep(1500);
    mark('hero');
    await sleep(6500);
    await glide(page, '#problem', 1600, 60);
    mark('problem');
    await sleep(4500);
    await glide(page, '.story', 1600, 40);
    mark('story');
    // walk through the four sticky scenes
    const steps = await page.locator('.story-step').count();
    for (let i = 0; i < steps; i++) {
      await glide(page, `.story-step[data-i="${i}"]`, 1500, 260);
      mark(`story${i}`);
      await sleep(4000);
    }
    await glide(page, '.spot', 1600, 60);
    mark('spot');
    await sleep(1200);
    for (const n of [3, 10, 17]) {
      await page.locator('.tx-tile').nth(n).click();
      await sleep(1300);
    }
    mark('spot-done');
    await sleep(1500);
    await glide(page, '.sees', 1600, 40);
    mark('sees');
    await sleep(5000);
    await glide(page, '.chain', 1600, 40);
    mark('chain');
    await sleep(4500);
    await glide(page, '#live-slot', 1600, 120);
    mark('live');
    await sleep(3500);
    await glide(page, '.final', 1800, 0);
    mark('final');
    await sleep(5000);
  });
}

if (SEGMENTS.includes('playground')) {
  const JUDGE = process.env.JUDGE_CODE;
  if (!JUDGE) throw new Error('set JUDGE_CODE');
  await clip('playground', async (page, mark) => {
    await page.goto(`${BASE}/playground`);
    await page.getByRole('heading', { name: '1. Enter the judge code' }).waitFor();
    mark('code');
    await sleep(1200);
    await page.locator('#judge-code').pressSequentially(JUDGE, { delay: 40 });
    await page.getByRole('button', { name: 'Continue' }).click();
    await page.getByRole('heading', { name: '2. Become the heir' }).waitFor();
    mark('heir');
    await sleep(1200);
    await page.getByRole('button', { name: 'Create my heir key' }).click();
    await page.locator('#heir-code').waitFor();
    await sleep(2500);
    await page.getByRole('button', { name: 'Use this heir code' }).click();
    await page.getByRole('heading', { name: '3. Start a vault' }).waitFor();
    mark('start');
    await page.locator('input[name="minutes"][value="3"]').check();
    await sleep(1000);
    await page.getByRole('button', { name: 'Start the vault' }).click();
    await page.getByRole('heading', { name: '3. Your vault is running' }).waitFor({ timeout: 120_000 });
    mark('running');
    await page.locator('li[data-step="sealed"].done').waitFor({ timeout: 15 * 60_000 });
    mark('sealed');
    const flip = page.getByRole('button', { name: "Flip (I'm still here)" });
    await flip.waitFor();
    for (let i = 0; i < 60 && (await flip.isDisabled()); i++) await sleep(2000);
    await sleep(800);
    await flip.click();
    mark('flip');
    await page.locator('li[data-step="flip"].done').waitFor({ timeout: 10 * 60_000 });
    mark('flipped');
    for (let i = 0; i < 90; i++) {
      const s = page.getByRole('button', { name: 'Stop flipping' });
      if (await s.isEnabled().catch(() => false)) break;
      await sleep(2000);
    }
    await page.getByRole('button', { name: 'Stop flipping' }).click();
    mark('stopped');
    await page.getByRole('heading', { name: '4. Open the kit' }).waitFor({ timeout: 12 * 60_000 });
    mark('unlocked');
    await sleep(1500);
    await page.getByRole('button', { name: 'Open the kit' }).click();
    await page.getByRole('heading', { name: 'The kit is open' }).waitFor({ timeout: 120_000 });
    mark('opened');
    await glide(page, '#opened-title', 1400, 200);
    await sleep(3500);
    await page.getByRole('button', { name: 'Confirm' }).click();
    await page.locator('#outcome').filter({ hasText: 'The coins went back' }).waitFor({ timeout: 10 * 60_000 });
    mark('returned');
    await glide(page, '#outcome', 1400, 200);
    await sleep(4000);
  });
}

if (SEGMENTS.includes('owner')) {
  const URL = process.env.OWNER_URL;
  const HEIR = process.env.HEIR;
  if (!URL || !HEIR) throw new Error('set OWNER_URL and HEIR');
  await clip('owner', async (page, mark) => {
    await page.goto(URL);
    await page.getByRole('heading', { name: /Create your store|Unlock your store/ }).waitFor();
    mark('store');
    await sleep(1500);
    await page.getByRole('button', { name: 'Testnet (test coins)' }).click().catch(() => undefined);
    await sleep(800);
    const pass = 'video-demo-passphrase';
    await page.locator('#pass1').pressSequentially(pass, { delay: 25 });
    if (await page.locator('#pass2').isVisible()) {
      await page.locator('#pass2').pressSequentially(pass, { delay: 15 });
      await page.getByRole('button', { name: 'Create the store' }).click();
    } else {
      await page.getByRole('button', { name: 'Unlock' }).click();
    }
    await page.getByRole('heading', { name: 'New plan', exact: true }).waitFor({ timeout: 120_000 });
    mark('newplan');
    await sleep(1500);
    await page.locator('#code0').pressSequentially(HEIR, { delay: 8 });
    await page.selectOption('#every', '90d');
    await page.locator('#amount').pressSequentially('0.05', { delay: 60 });
    await page.locator('#message').pressSequentially('For Deniz. Look after each other.', { delay: 25 });
    await sleep(800);
    mark('submit');
    await page.getByRole('button', { name: 'Create the plan' }).click();
    await page.locator('#created-title').waitFor({ timeout: 180_000 });
    mark('created');
    await sleep(4500);
    await page.getByRole('button', { name: 'Done' }).click();
    await glide(page, '.plan-card', 1200, 160);
    mark('plan');
    await sleep(2500);
    await page.getByRole('button', { name: 'Calendar reminder' }).click();
    await sleep(1500);
    mark('calendar');
    await glide(page, '.app-flow', 900, 40);
    await page.getByRole('button', { name: 'New plan' }).click();
    await page.getByRole('button', { name: /Scheduled payments/ }).click();
    mark('schedule');
    await sleep(800);
    await page.getByLabel('Payee code 1').pressSequentially(HEIR, { delay: 6 });
    await page.getByLabel(/Each installment/).pressSequentially('0.02', { delay: 60 });
    await page.getByLabel('Number of installments').fill('12');
    await page.getByLabel('How often').selectOption('30d');
    await sleep(3500);
    mark('schedule-done');
  });
}

await browser.close();
