// Live rehearsal of the playground against a REAL keeper (not part of the test suite).
//
// Drives the real UI like a judge: judge code, heir key, a 3 minute vault, one
// flip, unlock, open the kit in the browser, return the coins. Records the time
// of every phase and every txid shown, and takes screenshots.
//
//   cd web && npx vite build
//   (start the keeper with SANDGLASS_WEB_DIR=web/dist, see docs/PLAYGROUND_API.md)
//   JUDGE_CODE=... node e2e-live/rehearsal.mjs
//
// Environment:
//   BASE         keeper origin            (default http://127.0.0.1:8787)
//   JUDGE_CODE   the judge code           (required)
//   VIEWPORT     desktop (1440x900) | phone (390x844)   (default desktop)
//   SHOTS        all | few                (default all on desktop, few on phone)
//   OUT          screenshot directory     (default test-results/rehearsal/<viewport>)
//   MINUTES      3 | 5 | 10               (default 3)
//   FLIPS        number of flips to do    (default 1)
//   HEADED=1     show the browser
//
// The report (phases, txids) is printed as JSON at the end and written to OUT/report.json.
import { chromium } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const BASE = (process.env.BASE ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
const JUDGE = process.env.JUDGE_CODE;
if (!JUDGE) throw new Error('set JUDGE_CODE');
const VIEWPORT = process.env.VIEWPORT ?? 'desktop';
const PHONE = VIEWPORT === 'phone';
const SHOTS = process.env.SHOTS ?? (PHONE ? 'few' : 'all');
const OUT = process.env.OUT ?? `test-results/rehearsal/${VIEWPORT}`;
const MINUTES = Number(process.env.MINUTES ?? 3);
const FLIPS = Number(process.env.FLIPS ?? 1);
mkdirSync(OUT, { recursive: true });

const t0 = Date.now();
const secs = () => Math.round((Date.now() - t0) / 1000);
const log = (m) => console.log(`[${String(secs()).padStart(4)}s] ${m}`);
const phases = {};
const mark = (name) => {
  phases[name] = secs();
  log(`PHASE ${name}`);
};
let n = 0;
async function shot(page, name, always = false) {
  if (SHOTS !== 'all' && !always) return;
  n += 1;
  const file = `${OUT}/${String(n).padStart(2, '0')}-${name}.png`;
  await page.screenshot({ path: file, fullPage: true });
  log(`screenshot ${file}`);
}

const browser = await chromium.launch({ headless: process.env.HEADED !== '1' });
const context = await browser.newContext(
  PHONE
    ? { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true, deviceScaleFactor: 2 }
    : { viewport: { width: 1440, height: 900 } },
);
const page = await context.newPage();
page.setDefaultTimeout(30_000);
const problems = [];
page.on('console', (m) => {
  if (m.type() === 'error') problems.push(`console: ${m.text()}`);
});
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
page.on('requestfailed', (r) => problems.push(`requestfailed: ${r.url()} ${r.failure()?.errorText}`));

// Log every change of the timeline, so the phases can be read from the log.
let lastSig = '';
const sampler = setInterval(async () => {
  try {
    const sig = await page.evaluate(() => {
      const steps = [...document.querySelectorAll('.timeline li')].map(
        (li) => `${li.getAttribute('data-step')}:${li.className.replace(/\s+/g, '.')}`,
      );
      const h = document.querySelector('h2')?.textContent ?? '';
      return `${h} | ${steps.join(' ')} | ${document.querySelector('#countdown')?.textContent ?? ''}`;
    });
    const base = sig.replace(/\| [^|]*$/, '');
    if (base !== lastSig) {
      lastSig = base;
      log(`UI ${sig}`);
    }
  } catch {
    /* page navigating */
  }
}, 2000);

const txids = new Set();
const txKinds = new Map(); // txid -> kind, from the session snapshots the keeper returns
page.on('response', async (r) => {
  if (!/\/v1\/playground\/sessions\/[0-9a-f]+(\/(flip|settle))?$/.test(r.url())) return;
  try {
    const j = await r.json();
    for (const t of j.txs ?? []) {
      if (!txKinds.has(t.txid)) {
        txKinds.set(t.txid, t.kind);
        txids.add(t.txid);
        log(`TX ${t.kind} ${t.txid} ${t.explorer ?? ''}`);
      }
    }
  } catch {
    /* not JSON */
  }
});
async function collectTxids() {
  const found = await page.evaluate(() => {
    const out = [];
    for (const li of document.querySelectorAll('.timeline li')) {
      const m = li.textContent?.match(/[0-9a-f]{64}/g) ?? [];
      const href = [...li.querySelectorAll('a')].map((a) => a.href);
      out.push({ step: li.getAttribute('data-step'), ids: m, href });
    }
    const all = document.body.innerText.match(/\b[0-9a-f]{64}\b/g) ?? [];
    return { steps: out, all };
  });
  for (const id of found.all) txids.add(id);
  return found;
}

try {
  await page.goto(`${BASE}/playground`);
  await page.getByRole('heading', { name: '1. Enter the judge code' }).waitFor();
  mark('page_loaded');
  log(`status line: ${await page.locator('#status-line').innerText()}`);
  await shot(page, 'judge-code');
  await page.getByLabel('Judge code').fill(JUDGE);
  await page.getByRole('button', { name: 'Continue' }).click();

  await page.getByRole('heading', { name: '2. Become the heir' }).waitFor();
  await page.getByRole('button', { name: 'Create my heir key' }).click();
  await page.locator('.phrase li').nth(23).waitFor();
  const heirCode = (await page.locator('#heir-code').innerText()).trim();
  log(`heir code ${heirCode.slice(0, 12)}...`);
  await shot(page, 'heir-key');
  await page.getByRole('button', { name: 'Use this heir code' }).click();

  await page.getByRole('heading', { name: '3. Start a vault' }).waitFor();
  await shot(page, 'start-vault');
  await page.getByLabel(`${MINUTES} minutes`).check();
  await page.getByRole('button', { name: 'Start the vault' }).click();
  mark('vault_started');
  await page.getByRole('heading', { name: '3. Your vault is running' }).waitFor();
  await shot(page, 'funding');

  // Funding transaction broadcast, then confirmed.
  await page.locator('li[data-step="funded"] a, li[data-step="funded"].done').first().waitFor({ timeout: 10 * 60_000 });
  mark('fund_tx_shown');
  await shot(page, 'fund-tx-shown');
  await page.locator('li[data-step="funded"].done').waitFor({ timeout: 15 * 60_000 });
  await page.locator('li[data-step="sealed"].done').waitFor({ timeout: 5 * 60_000 });
  mark('funded_and_sealed');
  await shot(page, 'sealed', true);

  for (let i = 1; i <= FLIPS; i++) {
    const flip = page.getByRole('button', { name: "Flip (I'm still here)" });
    await flip.waitFor();
    await page.waitForFunction(
      () => ![...document.querySelectorAll('button')].find((b) => b.textContent?.includes("Flip (I'm still here)"))?.disabled,
      null,
      { timeout: 10 * 60_000 },
    );
    mark(`flip${i}_enabled`);
    await flip.click();
    mark(`flip${i}_pressed`);
    await shot(page, `flip${i}-pressed`);
    await page.locator('li[data-step="flip"].done').waitFor({ timeout: 10 * 60_000 });
    await page.getByText(`Flipped ${i} time`).first().waitFor({ timeout: 5 * 60_000 });
    mark(`flip${i}_reflected`);
    await shot(page, `flip${i}-done`, true);
    // Re-seal: the button is enabled again once the swept funds are confirmed.
    await page.waitForFunction(
      () => ![...document.querySelectorAll('button')].find((b) => b.textContent?.includes("Flip (I'm still here)"))?.disabled,
      null,
      { timeout: 10 * 60_000 },
    );
    mark(`flip${i}_resealed_confirmed`);
    await shot(page, `flip${i}-resealed`);
  }

  await page.getByRole('button', { name: 'Stop flipping' }).click();
  await shot(page, 'stopped-flipping');
  mark('stopped_flipping');

  await page.getByRole('heading', { name: '4. Open the kit' }).waitFor({ timeout: (MINUTES + 6) * 60_000 });
  mark('unlocked');
  await shot(page, 'unlocked', true);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await page.getByRole('heading', { name: 'The kit is open' }).waitFor({ timeout: 120_000 });
  mark('kit_opened');
  await shot(page, 'kit-open', true);
  await page.getByRole('button', { name: 'Show the seed' }).click();
  await shot(page, 'seed-shown');
  await page.getByRole('button', { name: 'Hide the seed' }).click();

  await page.getByLabel('Return the coins to the demo').check();
  await page.getByRole('button', { name: 'Confirm' }).click();
  mark('return_requested');
  await shot(page, 'settling');
  await page.locator('#outcome').filter({ hasText: 'The coins went back to the demo wallet' }).waitFor({ timeout: 10 * 60_000 });
  mark('returned');
  const final = await collectTxids();
  await shot(page, 'final', true);
  log(`final txs: ${JSON.stringify(final.steps)}`);
  console.log('OUTCOME_OK');
} catch (e) {
  log(`FAILED: ${e.message}`);
  await shot(page, 'failure', true).catch(() => {});
  process.exitCode = 1;
} finally {
  clearInterval(sampler);
  await collectTxids().catch(() => {});
  const report = { base: BASE, viewport: VIEWPORT, minutes: MINUTES, flips: FLIPS, phases, txids: Object.fromEntries(txKinds), problems };
  writeFileSync(`${OUT}/report.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
  await browser.close();
}
