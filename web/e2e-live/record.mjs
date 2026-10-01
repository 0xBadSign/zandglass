// Demo-video recorder: drives the real UI against a running keeper and records
// raw clips with Playwright `recordVideo` (1280x720, dark). Not part of the test suite.
//
//   cd web && npx vite build   (start the keeper with SANDGLASS_WEB_DIR=web/dist)
//   JUDGE_CODE=... node e2e-live/record.mjs
//
// Environment:
//   BASE         keeper origin            (default http://127.0.0.1:8787)
//   JUDGE_CODE   the judge code           (required)
//   OUT          output directory         (default ../.private/video/raw)
//   MINUTES      3 | 5 | 10               (default 3)
//   SEGMENTS     comma list of landing,heir,playground,cards (default all)
//   HEADED=1     show the browser
//
// Output: OUT/landing.webm, heir.webm, playground.webm, cards/*.png and
// OUT/manifest.json (seconds from the start of each clip for every phase, txids).
// The judge code and every phrase shown are throwaway test values: do not record
// real secrets. For a mainnet recording, review the clips before publishing.
import { chromium } from '@playwright/test';
import { mkdirSync, writeFileSync, renameSync, readdirSync, readFileSync, existsSync } from 'node:fs';

const BASE = (process.env.BASE ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
const JUDGE = process.env.JUDGE_CODE;
if (!JUDGE) throw new Error('set JUDGE_CODE');
const OUT = process.env.OUT ?? '../.private/video/raw';
const MINUTES = Number(process.env.MINUTES ?? 3);
const SEGMENTS = (process.env.SEGMENTS ?? 'landing,heir,playground,cards').split(',');
const SIZE = { width: 1280, height: 720 };
mkdirSync(`${OUT}/cards`, { recursive: true });
const manifest = existsSync(`${OUT}/manifest.json`) ? JSON.parse(readFileSync(`${OUT}/manifest.json`, 'utf8')) : { clips: {} };
manifest.base = BASE;
manifest.minutes = MINUTES;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const browser = await chromium.launch({ headless: process.env.HEADED !== '1' });

// One clip = one browser context = one video file.
async function clip(name, body) {
  const dir = `${OUT}/_tmp_${name}`;
  const context = await browser.newContext({
    viewport: SIZE,
    deviceScaleFactor: 1,
    colorScheme: 'dark',
    reducedMotion: 'no-preference',
    recordVideo: { dir, size: SIZE },
  });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  const t0 = Date.now();
  const marks = {};
  const mark = (k) => {
    marks[k] = Number(((Date.now() - t0) / 1000).toFixed(1));
    console.log(`[${name} ${marks[k]}s] ${k}`);
  };
  let extra = {};
  try {
    extra = (await body(page, mark)) ?? {};
  } catch (e) {
    console.log(`[${name}] FAILED: ${e.message}`);
    process.exitCode = 1;
  }
  mark('end');
  await context.close();
  const f = readdirSync(dir).find((x) => x.endsWith('.webm'));
  if (f) renameSync(`${dir}/${f}`, `${OUT}/${name}.webm`);
  manifest.clips[name] = { marks, ...extra };
}

async function smoothScrollTo(page, y, ms) {
  await page.evaluate(
    ([target, dur]) =>
      new Promise((res) => {
        const start = window.scrollY;
        const t0 = performance.now();
        const step = (now) => {
          const k = Math.min(1, (now - t0) / dur);
          const e = k < 0.5 ? 2 * k * k : 1 - Math.pow(-2 * k + 2, 2) / 2;
          window.scrollTo(0, start + (target - start) * e);
          if (k < 1) requestAnimationFrame(step);
          else res();
        };
        requestAnimationFrame(step);
      }),
    [y, ms],
  );
}

if (SEGMENTS.includes('landing')) {
  await clip('landing', async (page, mark) => {
    await page.goto(`${BASE}/`);
    await page.getByRole('heading', { level: 1 }).waitFor();
    await sleep(3500);
    mark('hero');
    for (const id of ['proof', 'how', 'leak', 'real']) {
      const y = await page.evaluate((i) => document.getElementById(i).getBoundingClientRect().top + window.scrollY - 70, id);
      await smoothScrollTo(page, y, 2200);
      mark(id);
      await sleep(id === 'proof' ? 5000 : 4500);
    }
  });
}

if (SEGMENTS.includes('heir')) {
  await clip('heir', async (page, mark) => {
    await page.goto(`${BASE}/heir.html`);
    await page.getByRole('heading', { name: 'Become an heir' }).waitFor();
    await sleep(2500);
    mark('heir_start');
    await page.getByRole('button', { name: 'Create my words' }).click();
    await page.locator('.phrase li').nth(23).waitFor();
    mark('words');
    await sleep(1500);
    await smoothScrollTo(page, 200, 1200);
    await sleep(4500);
    const words = await page.locator('.phrase li').allInnerTexts();
    await page.getByRole('button', { name: 'I wrote them down' }).click();
    await page.getByRole('heading', { name: 'Check your paper' }).waitFor();
    const text = await page.locator('main').innerText();
    const positions = text.match(/positions ([\d, ]+) from/)[1].split(',').map((s) => Number(s.trim()));
    await sleep(1200);
    for (const p of positions) {
      await page.locator(`#w${p}`).pressSequentially(words[p - 1].trim(), { delay: 70 });
    }
    await sleep(800);
    await page.getByRole('button', { name: 'Check my words' }).click();
    await page.getByRole('heading', { name: 'Your heir code' }).waitFor();
    mark('heir_code');
    await sleep(2500);
    await smoothScrollTo(page, 470, 1500);
    await sleep(5500);
  });
}

if (SEGMENTS.includes('playground')) {
  const txKinds = {};
  await clip('playground', async (page, mark) => {
    page.on('response', async (r) => {
      if (!/\/v1\/playground\/sessions\/[0-9a-f]+(\/(flip|settle))?$/.test(r.url())) return;
      try {
        for (const t of (await r.json()).txs ?? []) txKinds[t.txid] ??= t.kind;
      } catch {
        /* not JSON */
      }
    });
    await page.goto(`${BASE}/playground.html`);
    await page.getByRole('heading', { name: '1. Enter the judge code' }).waitFor();
    await sleep(2500);
    mark('judge_code');
    await page.getByLabel('Judge code').pressSequentially(JUDGE, { delay: 50 });
    await sleep(600);
    await page.getByRole('button', { name: 'Continue' }).click();
    await page.getByRole('heading', { name: '2. Become the heir' }).waitFor();
    await sleep(1500);
    await page.getByRole('button', { name: 'Create my heir key' }).click();
    await page.locator('.phrase li').nth(23).waitFor();
    mark('heir_key');
    await sleep(4500);
    await page.getByRole('button', { name: 'Use this heir code' }).click();
    await page.getByRole('heading', { name: '3. Start a vault' }).waitFor();
    mark('start_vault_form');
    await sleep(2500);
    await page.getByLabel(`${MINUTES} minutes`).check();
    await sleep(1200);
    await page.getByRole('button', { name: 'Start the vault' }).click();
    mark('vault_started');
    await page.getByRole('heading', { name: '3. Your vault is running' }).waitFor();
    await sleep(2000);
    await page.locator('li[data-step="funded"] a, li[data-step="funded"].done').first().waitFor({ timeout: 10 * 60_000 });
    mark('fund_tx_shown');
    await sleep(2500);
    await page.locator('li[data-step="funded"].done').waitFor({ timeout: 15 * 60_000 });
    await page.locator('li[data-step="sealed"].done').waitFor({ timeout: 5 * 60_000 });
    mark('sealed');
    await sleep(3500);
    const flip = page.getByRole('button', { name: "Flip (I'm still here)" });
    await flip.waitFor();
    await page.waitForFunction(
      () => ![...document.querySelectorAll('button')].find((b) => b.textContent?.includes("Flip (I'm still here)"))?.disabled,
      null,
      { timeout: 10 * 60_000 },
    );
    mark('flip_enabled');
    await sleep(1200);
    await flip.click();
    mark('flip_pressed');
    await sleep(2000);
    await page.locator('li[data-step="flip"].done').waitFor({ timeout: 10 * 60_000 });
    await page.getByText('Flipped 1 time').first().waitFor({ timeout: 5 * 60_000 });
    mark('flip_done');
    await sleep(3500);
    await page.waitForFunction(
      () => ![...document.querySelectorAll('button')].find((b) => b.textContent?.includes("Flip (I'm still here)"))?.disabled,
      null,
      { timeout: 10 * 60_000 },
    );
    mark('resealed');
    await sleep(1500);
    await page.getByRole('button', { name: 'Stop flipping' }).click();
    mark('stopped_flipping');
    await sleep(3000);
    await page.getByRole('heading', { name: '4. Open the kit' }).waitFor({ timeout: (MINUTES + 8) * 60_000 });
    mark('unlocked');
    await sleep(3500);
    await page.getByRole('button', { name: 'Open the kit' }).click();
    await page.getByRole('heading', { name: 'The kit is open' }).waitFor({ timeout: 120_000 });
    mark('kit_opened');
    await sleep(3500);
    await page.getByRole('button', { name: 'Show the seed' }).click();
    await sleep(3500);
    await page.getByRole('button', { name: 'Hide the seed' }).click();
    await sleep(800);
    await page.getByLabel('Return the coins to the demo').check();
    await sleep(1000);
    await page.getByRole('button', { name: 'Confirm' }).click();
    mark('return_requested');
    await page
      .locator('#outcome')
      .filter({ hasText: 'The coins went back to the demo wallet' })
      .waitFor({ timeout: 10 * 60_000 });
    mark('returned');
    await sleep(2500);
    // Scroll the timeline into view so every txid is readable.
    await page.locator('.timeline').first().scrollIntoViewIfNeeded();
    await sleep(5500);
    mark('txids_visible');
  });
  manifest.clips.playground.txids = txKinds;
}

// Text cards: rendered with the site's own CSS and fonts, screenshotted as PNG.
if (SEGMENTS.includes('cards')) {
  const idx = await (await fetch(`${BASE}/index.html`)).text();
  const css = [...idx.matchAll(/href="([^"]+\.css)"/g)].map((m) => `<link rel="stylesheet" href="${m[1]}">`).join('');
  const cards = {
    title: `<p class="k">SANDGLASS</p><h1>Pass on your shielded ZEC. <em>Privately.</em></h1><p class="s">Time-locked inheritance for Zcash. A dead man's switch with nothing on chain.</p>`,
    problem: `<p class="k">THE PROBLEM</p><h2>Shielded Zcash has <em>no timelocks.</em></h2><ul><li>No private way to leave your coins to someone.</li><li>Heirs cannot even find shielded funds without keys.</li><li>Bitcoin-style inheritance publishes the plan on chain.</li></ul>`,
    leak: `<p class="k">WHY IT DOES NOT LEAK</p><h2>Nothing on chain <em>but ordinary transactions.</em></h2><ul><li>Fund, flip and claim are plain shielded transactions.</li><li>No contract, no script, no heir address, no amounts.</li><li>The keeper is a blind mailbox: it cannot open a kit.</li></ul>`,
    real: `<p class="k">WHAT IS REAL, WHAT IS A MOCK</p><h2>Real <em>crypto.</em> One mocked step.</h2><ul><li>Real: age + drand timelock, real Zcash transactions, real browser claim.</li><li>Mock: the playground lends demo coins, then takes them back.</li><li>Not audited. Testnet in this draft; mainnet for the final.</li></ul>`,
    cta: `<p class="k">EVIDENCE</p><h2>Testnet and mainnet txids are in <em>docs/evidence</em></h2><p class="s">Try it yourself in the playground. The judge code is in the submission description.</p><p class="s url">Source, docs and evidence: the Sandglass repository</p>`,
  };
  const context = await browser.newContext({ viewport: SIZE, deviceScaleFactor: 1, colorScheme: 'dark' });
  const page = await context.newPage();
  for (const [name, inner] of Object.entries(cards)) {
    await page.route(`${BASE}/__card.html`, (route) =>
      route.fulfill({
        contentType: 'text/html',
        body: `<!doctype html><html><head><meta charset="utf-8">${css}<style>
body{margin:0;width:1280px;height:720px;display:flex;align-items:center;background:radial-gradient(900px 500px at 80% 10%,rgba(244,183,40,.13),transparent),#07080a;color:#ece9e1;font:400 24px/1.5 var(--sans)}
main{padding:0 110px;max-width:1060px}
.k{font:600 16px var(--mono);letter-spacing:.2em;color:#f4b728;margin:0 0 28px}
h1,h2{font:500 64px/1.12 var(--serif);margin:0 0 30px;letter-spacing:-.01em}
h1{font-size:76px} em{color:#f4b728;font-style:italic}
.s{color:#a7a296;font-size:26px;margin:10px 0}.url{font:500 20px var(--mono);color:#e9cd8f;margin-top:40px}
ul{list-style:none;padding:0;margin:0}li{margin:14px 0;padding-left:30px;position:relative;color:#d8d4c8;font-size:27px}
li::before{content:"";position:absolute;left:0;top:.62em;width:10px;height:10px;border-radius:50%;background:#f4b728}
</style></head><body><main>${inner}</main></body></html>`,
      }),
    );
    await page.goto(`${BASE}/__card.html`);
    await page.evaluate(() => document.fonts.ready);
    await sleep(500);
    await page.screenshot({ path: `${OUT}/cards/${name}.png` });
    await page.unroute(`${BASE}/__card.html`);
  }
  await context.close();
}

await browser.close();
writeFileSync(`${OUT}/manifest.json`, JSON.stringify(manifest, null, 2));
console.log(JSON.stringify(manifest, null, 2));
