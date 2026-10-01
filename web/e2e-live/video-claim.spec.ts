// Records the claim page's guided restore for the demo video (not part of the
// test suite; run with e2e-live/video.config.ts). Uses the TEST ONLY kit and
// the recorded drand beacon, so nothing real is shown.
import { test } from '@playwright/test';
import { makeJsonKit, read, recordedBeacon } from '../src/test-helpers';

const PHRASE = read('TEST_ONLY_phrase.txt').trim();

const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';

test('video: claim and guided restore', async ({ page }) => {
  test.setTimeout(120_000);
  const beacon = recordedBeacon();
  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.route(`https://${host}/**`, (route) =>
      route.request().url().endsWith(`/${CHAIN}/public/${beacon.round}`)
        ? route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(beacon) })
        : route.fulfill({ status: 404, body: '' }),
    );
  }
  const kit = await makeJsonKit();
  await page.goto('/claim');
  await page.waitForTimeout(1200);
  await page.locator('#kit-file').setInputFiles({ name: 'plan-e3.zandglass', mimeType: 'application/json', buffer: Buffer.from(kit) });
  await page.waitForTimeout(1500);
  await page.getByLabel('Your 24 words').pressSequentially(PHRASE.split(' ').slice(0, 24).join(' '), { delay: 4 });
  await page.waitForTimeout(500);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await page.getByRole('heading', { name: 'Your inheritance is open' }).waitFor();
  await page.waitForTimeout(1500);
  await page.locator('.restore-guide').scrollIntoViewIfNeeded();
  await page.evaluate(() => window.scrollBy(0, -120));
  await page.waitForTimeout(1500);
  for (const step of ['2. Restore', '3. Birthday', '4. Move the funds']) {
    await page.getByRole('button', { name: step }).click();
    await page.waitForTimeout(1600);
  }
});
