import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const SHOTS = 'test-results/screenshots';
const PITCH =
  'Your heir can open your vault only after you stop checking in. Until then nobody can, and nothing on chain reveals the plan, the heir or the amounts.';

async function axe(page: Page): Promise<void> {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    const bad = r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(bad.map((v) => `${scheme}: ${v.id} ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([]);
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

const liveVault = (claimTx: string | null, ms = 2 * 86_400_000 + 3_600_000) =>
  JSON.stringify({ network: 'main', unlock_time: new Date(Date.now() + ms).toISOString(), claim_tx: claimTx });

test('landing: content, links, live vault, axe, no horizontal scroll', async ({ page }, info) => {
  const hosts = new Set<string>();
  page.on('request', (r) => hosts.add(new URL(r.url()).host));
  await page.route('**/live-vault.json', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: liveVault(null) }),
  );
  await page.goto('/index.html');

  await expect(page.getByRole('heading', { level: 1, name: 'Pass on your shielded ZEC. Privately.' })).toBeVisible();
  await expect(page.locator('.hero3 .lead')).toHaveText(PITCH);
  await expect(page.locator('#problem-h')).toContainText('Shielded Zcash has no timelocks.');

  // Four steps, in order.
  await expect(page.locator('.story-step h3')).toHaveText([
    'Your heir makes a key',
    'Sandglass seals a vault',
    'You flip, now and then',
    'The sand runs out',
  ]);
  // Who sees what.
  await expect(page.locator('.sees-grid')).toContainText('The mailbox keeper');
  await expect(page.locator('.sees-grid')).toContainText('Someone who steals a kit');
  // Spot the inheritance: every tile is an ordinary shielded transaction.
  await page.locator('.tx-tile').first().click();
  await expect(page.locator('.spot-msg')).toContainText('Ordinary shielded transaction');
  // Real vs mock.
  await expect(page.getByRole('heading', { name: 'Real', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Mock or limited' })).toBeVisible();

  // Links.
  const href = async (name: string, nth = 0) => page.getByRole('link', { name, exact: true }).nth(nth).getAttribute('href');
  expect(await href('Try the playground')).toBe('./playground.html');
  expect(await href('Become an heir')).toBe('./heir.html');
  expect(await href('Claim')).toBe('./claim.html');
  expect(await href('Source code')).toBe('https://github.com/0xnesQuit/sandglass');
  expect(await href('Evidence')).toBe('https://github.com/0xnesQuit/sandglass/blob/main/docs/evidence/EVIDENCE.md');

  // The live vault card counts down.
  const card = page.locator('.live-card');
  await expect(card).toBeVisible();
  await expect(card.locator('.countdown')).toHaveText(/^2d 0[01]h /);
  await expect(card).toContainText('Mainnet');

  await axe(page);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  await page.screenshot({ path: `${SHOTS}/landing-${info.project.name}.png`, fullPage: true });

  // Nothing but this origin was contacted.
  expect([...hosts]).toEqual(['127.0.0.1:4173']);
});

test('landing: the live vault card is hidden without live-vault.json and shows a claim', async ({ page }) => {
  await page.goto('/index.html');
  await expect(page.getByRole('heading', { level: 1, name: 'Pass on your shielded ZEC. Privately.' })).toBeVisible();
  await expect(page.locator('.live-card')).toHaveCount(0);

  const tx = 'ab'.repeat(32);
  await page.route('**/live-vault.json', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: liveVault(tx, -60_000) }),
  );
  await page.reload();
  await expect(page.locator('.live-card .countdown')).toHaveText('Claimed');
  await expect(page.locator('.live-card')).toContainText(tx);
});

test('landing: motion stops under reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/index.html');
  const name = () => page.locator('.hero-stage .halo').evaluate((el) => getComputedStyle(el).animationName);
  expect(await name()).not.toBe('none');
  await expect(page.locator('canvas.dust')).toHaveCount(1);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.reload();
  expect(await name()).toBe('none');
  await expect(page.locator('canvas.dust')).toHaveCount(0);
});
