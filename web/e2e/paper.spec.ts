import { readFileSync } from 'node:fs';
import { expect, test, type Page, type Request, type Route } from '@playwright/test';
import { kitQrText } from '../src/lib/paper';
import { makeJsonKit, qrPng, recordedBeacon } from '../src/test-helpers';

const beacon = recordedBeacon();
const UNLOCK = 1692803367 + (beacon.round - 1) * 3;
const PLAN = '0123456789abcdef0123456789abcdef';
const ALLOWED_HOSTS = new Set(['127.0.0.1:4173', 'api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']);

/** Collect the hosts of every request (no kit data is ever sent anywhere). */
function watch(page: Page): Request[] {
  const seen: Request[] = [];
  page.on('request', (r) => seen.push(r));
  return seen;
}

function expectOnlyLocal(seen: Request[]): void {
  for (const r of seen) {
    if (r.url().startsWith('data:') || r.url().startsWith('blob:')) continue;
    expect(ALLOWED_HOSTS.has(new URL(r.url()).host), `unexpected host in ${r.url()}`).toBe(true);
    expect(r.method()).toBe('GET');
    expect(r.postData() ?? '').toBe('');
  }
}

async function pickPhoto(page: Page, png: Buffer): Promise<void> {
  await page.setInputFiles('#paper-photo', { name: 'paper-kit.png', mimeType: 'image/png', buffer: png });
}

test('claim page: a locked kit shows the heir pulse and downloads a calendar file', async ({ page }) => {
  await page.clock.setFixedTime(new Date((UNLOCK - 90_000) * 1000));
  const seen = watch(page);
  await page.goto('/claim');
  await page.setInputFiles('#kit-file', { name: 'k.zandglass', mimeType: 'application/json', buffer: Buffer.from(await makeJsonKit()) });
  await expect(page.getByRole('heading', { name: 'This kit is still locked' })).toBeVisible();
  await expect(page.locator('#pulse')).toContainText('The owner last checked in on');
  await expect(page.locator('#pulse')).toContainText('2026');
  await expect(page.locator('#pulse')).toContainText('(vault #3)');
  await expect(page.getByText('a newer kit replaces this one')).toBeVisible();

  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Add the unlock date to my calendar' }).click()]);
  expect(dl.suggestedFilename()).toMatch(/\.ics$/);
  const path = await dl.path();
  const ics = readFileSync(path, 'utf8');
  const d = new Date(UNLOCK * 1000).toISOString().replace(/[-:]/g, '').replace('.000', '');
  expect(ics).toContain(`DTSTART:${d}`);
  expect(ics).toContain('SUMMARY:Zandglass kit unlocks');
  expect(ics).toMatch(/UID:[0-9a-f]{32}@zandglass/);
  expect(ics.toLowerCase()).not.toContain(PLAN);
  expect(ics.match(/BEGIN:VALARM/g)).toHaveLength(1);
  expectOnlyLocal(seen);
});

test('claim page: a photo of a paper kit QR loads the kit', async ({ page }) => {
  await page.clock.setFixedTime(new Date((UNLOCK - 90_000) * 1000));
  const seen = watch(page);
  await page.goto('/claim');
  await expect(page.getByRole('heading', { name: 'Scan a paper kit' })).toBeVisible();
  await pickPhoto(page, qrPng(kitQrText(await makeJsonKit())));
  await expect(page.getByRole('heading', { name: 'This kit is still locked' })).toBeVisible();
  await expect(page.locator('.facts')).toContainText('#3');
  await expect(page.locator('.facts')).toContainText('Testnet');
  expectOnlyLocal(seen);
});

test('claim page: a picture without a QR code, or with a foreign one, is explained', async ({ page }) => {
  await page.goto('/claim');
  await pickPhoto(page, qrPng('hello, this is not a kit'));
  await expect(page.getByRole('alert')).toContainText('This kit looks damaged');
});

// --- owner panel ---------------------------------------------------------

const TOKEN = 'ab'.repeat(24);
const now = Math.floor(Date.now() / 1000);

const plan = {
  id: PLAN,
  short_id: '01234567',
  kind: 'legacy',
  status: 'active',
  epoch: 3,
  heirs: 1,
  period_secs: 90 * 86400,
  created: now - 3600,
  unlock_time: now + 89 * 86400,
  epoch_created: now - 86400,
  mailbox: true,
  pending: false,
  balance: { total_zat: 10_000_000, spendable_zat: 10_000_000, height: 3_500_000 },
  installments: null,
};

const state = { network: 'main', store: 'open', now, cap_zat: 25_000_000, public_keeper: 'https://zandglass.cash', plans: [plan] };

async function mockOwner(page: Page, kitText: string): Promise<void> {
  await page.route('**/api/**', async (route: Route) => {
    const path = new URL(route.request().url()).pathname;
    expect(route.request().headers()['authorization']).toBe(`Bearer ${TOKEN}`);
    const ok = (body: unknown) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/api/state') return ok(state);
    if (path === `/api/plans/${PLAN}/kit`) return ok({ file_name: '01234567-e3.zandglass', text: kitText });
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{"error":"not_found","message":"no"}' });
  });
}

test('owner: print paper kit shows one QR with the short id and unlock date, no amounts', async ({ page }) => {
  await mockOwner(page, await makeJsonKit());
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('button', { name: 'Print paper kit' }).click();
  const box = page.locator('#paper-kit');
  await expect(box.locator('svg')).toBeVisible();
  await expect(box.locator('svg')).toHaveCount(1);
  await expect(box).toContainText('01234567');
  await expect(box).toContainText('unlocks');
  await expect(box).toContainText('zandglass.cash/claim');
  await expect(box).not.toContainText('ZEC');
  await expect(box).not.toContainText('u1');
  await expect(page.getByRole('button', { name: 'Hide paper kit' })).toBeVisible();
  // The print stylesheet shows only the sheet.
  await page.emulateMedia({ media: 'print' });
  await page.evaluate(() => {
    const root = document.createElement('div');
    root.id = 'paper-print';
    const sheet = document.querySelector('.paper-sheet');
    if (sheet) root.append(sheet.cloneNode(true));
    document.body.append(root);
  });
  await expect(page.locator('#paper-print .paper-sheet svg')).toBeVisible();
  await expect(page.locator('main')).toBeHidden();
});

test('owner: a kit too big for one QR code offers the file instead', async ({ page }) => {
  const big = JSON.parse(await makeJsonKit()) as Record<string, unknown>;
  big['message'] = 'x'.repeat(4000);
  await mockOwner(page, JSON.stringify(big));
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('button', { name: 'Print paper kit' }).click();
  await expect(page.locator('#paper-kit')).toContainText('too big for one QR code');
  await expect(page.locator('#paper-kit svg')).toHaveCount(0);
});
