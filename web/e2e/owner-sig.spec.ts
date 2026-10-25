import { expect, test, type Page } from '@playwright/test';
import { read } from '../src/test-helpers';

// The signed test vector (docs/KIT_FORMAT.md section 8).
const SIGNED = read('TEST_ONLY_owner_kit.zandglass');

async function load(page: Page, kit: string, name = 'kit.zandglass'): Promise<void> {
  await page.setInputFiles('#kit-file', { name, mimeType: 'application/json', buffer: Buffer.from(kit) });
}

/** Block every network request except this app: the signature check is local. */
async function offline(page: Page): Promise<void> {
  await page.route(/^https:\/\//, (route) => route.abort('blockedbyclient'));
}

test('claim page: a signed kit is recognised, then the same key matches', async ({ page }) => {
  await offline(page);
  await page.goto('/claim');
  await load(page, SIGNED);
  await expect(page.locator('#owner-sig')).toContainText('Signed by the owner');
  await page.reload();
  await load(page, SIGNED);
  await expect(page.locator('#owner-sig')).toContainText('same owner');
});

test('claim page: an altered signed kit is refused', async ({ page }) => {
  await offline(page);
  await page.goto('/claim');
  const kit = JSON.parse(SIGNED) as Record<string, unknown>;
  kit['created'] = '2026-09-30T20:30:01Z';
  await load(page, JSON.stringify(kit, null, 2));
  await expect(page.getByText('This kit was altered')).toBeVisible();
  await expect(page.locator('#owner-sig')).toHaveCount(0);
});

test('claim page: a kit with its signature stripped warns after a signed one was seen', async ({ page }) => {
  await offline(page);
  await page.goto('/claim');
  await load(page, SIGNED);
  await expect(page.locator('#owner-sig')).toContainText('Signed by the owner');
  const kit = JSON.parse(SIGNED) as Record<string, unknown>;
  delete kit['owner_key'];
  delete kit['owner_sig'];
  await page.reload();
  await load(page, JSON.stringify(kit, null, 2));
  await expect(page.locator('#owner-sig')).toContainText('earlier kits of this plan were');
});
