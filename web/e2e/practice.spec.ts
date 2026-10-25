import { expect, test, type Page, type Request } from '@playwright/test';
import { roundTime } from '../src/drand';
import { recordedBeacon } from '../src/test-helpers';

const beacon = recordedBeacon();
const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';
const ALLOWED_HOSTS = new Set(['127.0.0.1:4173', 'api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']);
// The practice kit unlocks 60 s after it is made. Pin the clock so that round
// is exactly the recorded one: /heir "now" is 60 s before it, /claim "now" is after it.
const HEIR_NOW_MS = (roundTime(beacon.round) - 60) * 1000;
const CLAIM_NOW_MS = (roundTime(beacon.round) + 5) * 1000;

test('heir fire drill: create key, rehearse, open on claim, practice result', async ({ page }) => {
  const urls: string[] = [];
  page.on('request', (r: Request) => urls.push(r.url()));
  await pinClock(page);
  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.route(`https://${host}/**`, (route) => {
      if (route.request().url().endsWith(`/${CHAIN}/public/${beacon.round}`)) {
        return route.fulfill({
          status: 200,
          contentType: 'application/json',
          headers: { 'access-control-allow-origin': '*' },
          body: JSON.stringify(beacon),
        });
      }
      return route.fulfill({ status: 404, headers: { 'access-control-allow-origin': '*' }, body: '' });
    });
  }

  await page.goto('/heir');
  await page.getByRole('button', { name: 'Create my words' }).click();
  const words = await page.locator('.phrase li').allTextContents();
  expect(words).toHaveLength(24);
  await page.getByRole('button', { name: 'I wrote them down' }).click();
  const labels = await page.locator('label').allTextContents();
  for (const l of labels) {
    const p = Number(l.replace('Word ', ''));
    await page.getByLabel(`Word ${p}`, { exact: true }).fill(words[p - 1] ?? '');
  }
  await page.getByRole('button', { name: 'Check my words' }).click();
  await expect(page.locator('#heir-code')).toHaveText(/^age1[a-z0-9]{58}$/);

  await page.getByRole('button', { name: 'Make a practice kit' }).click();
  await expect(page.getByText('Your practice kit is ready')).toBeVisible();
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('button', { name: 'Download the practice kit' }).click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^practice-[0-9a-f]{8}\.zandglass$/);

  await page.getByRole('button', { name: 'Open it on the Claim page' }).click();
  await page.waitForURL('**/claim');
  await expect(page.getByRole('heading', { name: 'This kit is unlocked' })).toBeVisible();
  // The hand-over is read once and cleared.
  expect(await page.evaluate(() => sessionStorage.length)).toBe(0);

  await page.getByLabel('Your 24 words').fill(words.join(' '));
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'Practice complete' })).toBeVisible();
  await expect(page.getByText('This was a practice kit')).toBeVisible();
  await expect(page.getByText('No funds. You are ready.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Show the seed' })).toHaveCount(0);

  // Only this app and the drand relays were contacted; no phrase word was sent.
  for (const u of urls) {
    if (u.startsWith('data:') || u.startsWith('blob:')) continue;
    expect(ALLOWED_HOSTS.has(new URL(u).host), `unexpected host in ${u}`).toBe(true);
    if (!u.startsWith('http://127.0.0.1')) {
      for (const w of new Set(words)) expect(decodeURIComponent(u).includes(`/${w}`)).toBe(false);
    }
  }
});

async function pinClock(page: Page): Promise<void> {
  await page.addInitScript(
    ({ heirNow, claimNow }) => {
      const fixed = location.pathname.includes('claim') ? claimNow : heirNow;
      Date.now = () => fixed;
    },
    { heirNow: HEIR_NOW_MS, claimNow: CLAIM_NOW_MS },
  );
}
