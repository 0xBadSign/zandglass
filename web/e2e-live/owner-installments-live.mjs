// Live check of scheduled payments in the owner panel (`zandglass ui`).
// Not part of the test suite. Creates a schedule, funds it (FUND_CMD),
// starts it, saves the claim-links file, and leaves the plan running.
//
//   URL=http://127.0.0.1:8799/owner#t=... PAYEE=age1... OUT=... FUND_CMD="... {addr}" \
//   node e2e-live/owner-installments-live.mjs
import { chromium } from '@playwright/test';
import { execSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const URL = process.env.URL;
const PAYEE = process.env.PAYEE;
if (!URL || !PAYEE) throw new Error('set URL and PAYEE');
const OUT = process.env.OUT ?? 'test-results/owner-installments';
mkdirSync(OUT, { recursive: true });
const t0 = Date.now();
const log = (m) => console.log(`[${String(Math.round((Date.now() - t0) / 1000)).padStart(4)}s] ${m}`);
let n = 0;
const shot = async (page, name) => {
  const f = `${OUT}/${String(++n).padStart(2, '0')}-${name}.png`;
  await page.screenshot({ path: f, fullPage: true });
  log(`screenshot ${f}`);
};

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, acceptDownloads: true });
const page = await ctx.newPage();
try {
  await page.goto(URL);
  await page.getByRole('heading', { name: /Create your store|Unlock your store/ }).waitFor();
  const pass = 'owner-panel-test-passphrase';
  await page.fill('#pass1', pass);
  if (await page.getByRole('heading', { name: 'Create your store' }).isVisible()) {
    await page.fill('#pass2', pass);
    await page.getByRole('button', { name: 'Create the store' }).click();
  } else {
    await page.getByRole('button', { name: 'Unlock' }).click();
  }
  await page.getByRole('button', { name: 'Sync' }).waitFor({ timeout: 120_000 });
  if (!(await page.getByRole('heading', { name: 'New plan', exact: true }).isVisible())) {
    await page.getByRole('button', { name: 'New plan' }).click();
  }
  await page.getByRole('button', { name: /Scheduled payments/ }).click();
  await page.getByLabel('Payee code 1').fill(PAYEE);
  await page.getByLabel(/Each installment/).fill('0.001');
  await page.getByLabel('Number of installments').fill('2');
  await page.getByLabel('How often').selectOption('5m');
  await shot(page, 'form');
  await page.getByRole('button', { name: 'Create the schedule' }).click();
  await page.locator('#created-title').waitFor({ timeout: 180_000 });
  const addr = (await page.locator('.created-card .code-box').first().innerText()).trim();
  log(`schedule created, funding vault ${addr.slice(0, 16)}...`);
  await shot(page, 'created');
  if (process.env.FUND_CMD) {
    const out = execSync(process.env.FUND_CMD.replace('{addr}', addr), { encoding: 'utf8' });
    log(out.split('\n').filter((l) => /txid/i.test(l)).join(' | '));
  }
  await page.getByRole('button', { name: 'Done' }).click();

  for (let i = 0; i < 40; i++) {
    await page.getByRole('button', { name: 'Sync' }).click();
    await page.locator('.busy-line').waitFor({ state: 'detached', timeout: 300_000 });
    const confirming = await page.getByText('Funds are confirming').isVisible();
    const bal = await page.locator('.plan-card.sel .plan-stats dd').nth(1).innerText();
    log(`balance ${bal.replace(/\s+/g, ' ')}${confirming ? ' (confirming)' : ''}`);
    if (!/Press Sync|^0 /.test(bal) && !confirming) break;
    await page.waitForTimeout(30_000);
  }
  await shot(page, 'funded');
  for (let i = 0; i < 10; i++) {
    await page.getByRole('button', { name: 'Start the schedule' }).click();
    await page.locator('.busy-line').waitFor({ state: 'detached', timeout: 600_000 });
    if (await page.locator('#split-title').isVisible()) break;
    log(`start: ${(await page.locator('.notice').first().innerText().catch(() => '')).replace(/\s+/g, ' ').slice(0, 160)}`);
    await page.waitForTimeout(30_000);
  }
  log(`split: ${(await page.locator('.created-card p').first().innerText()).slice(0, 140)}`);
  await shot(page, 'started');
  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Download the claim links' }).click()]);
  await dl.saveAs(`${OUT}/${dl.suggestedFilename()}`);
  log(`claim links saved: ${OUT}/${dl.suggestedFilename()}`);
  log('INSTALLMENTS_OK');
} catch (e) {
  log(`FAILED: ${e}`);
  await shot(page, 'failure');
} finally {
  await browser.close();
}
