// Live check of the owner panel (`zandglass ui`) against a real network.
// Not part of the test suite. Drives the page like an owner: create the
// store, create a plan, wait for funding (done outside), sync, flip, download
// the kit and the calendar file, cancel back to an address.
//
//   URL=http://127.0.0.1:8799/owner#t=... HEIR=age1... RETURN_TO=utest1... \
//   OUT=../.private/design/owner FUND_CMD="..." node e2e-live/owner-live.mjs
//
// FUND_CMD (optional) is run once the funding address is known; "{addr}" is
// replaced by the vault address.
import { chromium } from '@playwright/test';
import { execSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';

const URL = process.env.URL;
const HEIR = process.env.HEIR;
if (!URL || !HEIR) throw new Error('set URL and HEIR');
const OUT = process.env.OUT ?? 'test-results/owner-live';
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
page.on('console', (m) => m.type() === 'error' && log(`console: ${m.text()}`));
try {
  await page.goto(URL);
  await page.getByRole('heading', { name: /Create your store|Unlock your store/ }).waitFor();
  await shot(page, 'store');
  const pass = 'owner-panel-test-passphrase';
  if (await page.getByRole('heading', { name: 'Create your store' }).isVisible()) {
    await page.fill('#pass1', pass);
    await page.fill('#pass2', pass);
    await page.getByRole('button', { name: 'Create the store' }).click();
  } else {
    await page.fill('#pass1', pass);
    await page.getByRole('button', { name: 'Unlock' }).click();
  }
  await page.getByRole('heading', { name: 'New inheritance plan' }).waitFor({ timeout: 120_000 });
  await shot(page, 'new-plan');
  await page.fill('#heir0', HEIR);
  await page.selectOption('#every', '10m');
  await page.fill('#amount', '0.001');
  await page.fill('#message', 'Owner panel live check.');
  await page.getByRole('button', { name: 'Create the plan' }).click();
  await page.locator('#created-title').waitFor({ timeout: 180_000 });
  const addr = (await page.locator('.created-card .code-box').first().innerText()).trim();
  log(`plan created, vault ${addr.slice(0, 16)}...`);
  await shot(page, 'created');

  if (process.env.FUND_CMD) {
    log('funding...');
    const out = execSync(process.env.FUND_CMD.replace('{addr}', addr), { encoding: 'utf8' });
    log(out.split('\n').filter((l) => /txid/i.test(l)).join(' | '));
  }
  await page.getByRole('button', { name: 'Done' }).click();

  // Sync until the balance shows and the funds are spendable (flip refuses before).
  for (let i = 0; i < 40; i++) {
    await page.getByRole('button', { name: 'Sync' }).click();
    await page.locator('.busy-line').waitFor({ state: 'detached', timeout: 300_000 });
    const bal = await page.locator('.plan-stats dd').nth(1).innerText();
    log(`balance: ${bal}`);
    if (!/Not synced|^0 /.test(bal)) break;
    await page.waitForTimeout(20_000);
  }
  await shot(page, 'funded');

  // Flip; retry while the funds are not yet confirmed.
  for (let i = 0; i < 20; i++) {
    await page.getByRole('button', { name: "Flip (I'm still here)" }).click();
    await page.locator('.busy-line').waitFor({ state: 'detached', timeout: 600_000 });
    const msg = await page.locator('.notice').first().innerText().catch(() => '');
    log(`flip: ${msg.replace(/\s+/g, ' ').slice(0, 200)}`);
    if (/Flipped plan/.test(msg)) break;
    await page.waitForTimeout(30_000);
  }
  await shot(page, 'flipped');

  const [kit] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Download kit' }).click()]);
  await kit.saveAs(`${OUT}/${kit.suggestedFilename()}`);
  log(`kit downloaded: ${kit.suggestedFilename()}`);
  const [cal] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Calendar reminder' }).click()]);
  await cal.saveAs(`${OUT}/${cal.suggestedFilename()}`);
  log(`calendar downloaded: ${cal.suggestedFilename()}`);

  if (process.env.RETURN_TO) {
    for (let i = 0; i < 20; i++) {
      await page.getByRole('button', { name: 'Cancel plan' }).click();
      await page.locator('.cancel-box input').fill(process.env.RETURN_TO);
      await page.getByRole('button', { name: 'Cancel and withdraw' }).click();
      await page.locator('.busy-line').waitFor({ state: 'detached', timeout: 600_000 });
      const msg = await page.locator('.notice').first().innerText().catch(() => '');
      log(`cancel: ${msg.replace(/\s+/g, ' ').slice(0, 200)}`);
      if (/cancelled/.test(msg)) break;
      await page.waitForTimeout(30_000);
    }
    await shot(page, 'cancelled');
  }
  log('OWNER_OK');
} catch (e) {
  log(`FAILED: ${e}`);
  await shot(page, 'failure');
} finally {
  await browser.close();
}
