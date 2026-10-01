import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

const SHOTS = 'test-results/screenshots';
const PITCH =
  'Shielded Zcash has no timelocks, so there is no private way to pass it on. ' +
  'Sandglass gives your vault a time-locked recovery path: your heir can open it only after you stop checking in ' +
  '— and nothing about it ever appears on chain.';

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
  await expect(page.locator('.lead')).toHaveText(PITCH);

  // Four steps, in order.
  await expect(page.locator('.how li h3')).toHaveText(['Heir code', 'Seal', 'Flip', 'The sand runs out']);
  // Who learns what.
  await expect(page.getByRole('table')).toContainText('The mailbox keeper');
  await expect(page.getByRole('table')).toContainText('Someone who steals a kit');
  // Real vs mock.
  await expect(page.getByRole('heading', { name: 'Real', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Mock or limited' })).toBeVisible();

  // Links.
  const href = async (name: string, nth = 0) => page.getByRole('link', { name, exact: true }).nth(nth).getAttribute('href');
  expect(await href('Try the playground')).toBe('./playground.html');
  expect(await href('Become an heir')).toBe('./heir.html');
  expect(await href('Claim')).toBe('./claim.html');
  expect(await href('GitHub repository')).toBe('https://github.com/0xnesQuit/sandglass');
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

test('landing: the hourglass stops animating under reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.goto('/index.html');
  const name = () => page.locator('.sg-art').evaluate((el) => getComputedStyle(el).animationName);
  expect(await name()).not.toBe('none');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await name()).toBe('none');
});
