import { createHash } from 'node:crypto';
import { expect, test, type Page } from '@playwright/test';
import { TEST_PAYLOAD, makeJsonKit, read, recordedBeacon } from '../src/test-helpers';

// The signed test vector (docs/KIT_FORMAT.md section 8) and the claim link
// owner hint of its key: the first 16 bytes of SHA-256 of the public key.
const SIGNED = read('TEST_ONLY_owner_kit.zandglass');
const PUBLIC = /^public=([0-9a-f]{64})$/m.exec(read('TEST_ONLY_owner_key.txt'))?.[1] ?? '';
const HINT = createHash('sha256').update(Buffer.from(PUBLIC, 'hex')).digest('hex').slice(0, 32);
const WRONG_HINT = 'f'.repeat(32);
const PHRASE = read('TEST_ONLY_phrase.txt').trim();
const beacon = recordedBeacon();
const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';

const unsigned = (): string => {
  const kit = JSON.parse(SIGNED) as Record<string, unknown>;
  delete kit['owner_key'];
  delete kit['owner_sig'];
  return JSON.stringify(kit);
};

/** Serve `kit` from mailbox `mb1` and the recorded beacon; nothing else leaves the machine. */
async function mailbox(page: Page, kit: string | Promise<string>): Promise<void> {
  await page.route('**/v1/mailboxes/mb1/kit', async (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: await kit }),
  );
  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.route(`https://${host}/**`, (route) =>
      route.request().url().endsWith(`/${CHAIN}/public/${beacon.round}`)
        ? route.fulfill({
            status: 200,
            contentType: 'application/json',
            headers: { 'access-control-allow-origin': '*' },
            body: JSON.stringify(beacon),
          })
        : route.fulfill({ status: 404, headers: { 'access-control-allow-origin': '*' }, body: '' }),
    );
  }
}

async function expectRefused(page: Page): Promise<void> {
  await expect(page.getByText('This kit was not sealed by the owner who sent you this link').first()).toBeVisible();
  // The kit was dropped: there is nothing to open.
  await expect(page.locator('#owner-sig')).toHaveCount(0);
  await expect(page.locator('#phrase')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Open the kit' })).toHaveCount(0);
}

test('claim link with the matching owner hint opens the kit', async ({ page }) => {
  await mailbox(page, SIGNED);
  await page.goto(`/claim#m=mb1&r=readtoken&o=${HINT}`);
  await expect(page.locator('#owner-sig')).toContainText('Signed by the owner your link names');
  await expect(page.locator('#owner-sig')).toHaveClass(/ok/);
});

test('claim link with a hint that names another key is a hard error', async ({ page }) => {
  await mailbox(page, SIGNED);
  await page.goto(`/claim#m=mb1&r=readtoken&o=${WRONG_HINT}`);
  await expectRefused(page);
  await expect(page.getByText('different key')).toBeVisible();
});

test('claim link with a hint refuses an unsigned kit', async ({ page }) => {
  await mailbox(page, unsigned());
  await page.goto(`/claim#m=mb1&r=readtoken&o=${HINT}`);
  await expectRefused(page);
  await expect(page.getByText('carries no signature')).toBeVisible();
});

test('claim link with a malformed hint is an incomplete link, never ignored', async ({ page }) => {
  await mailbox(page, SIGNED);
  await page.goto('/claim#m=mb1&r=readtoken&o=zz');
  await expect(page.getByText('This claim link is incomplete')).toBeVisible();
  await expect(page.locator('#owner-sig')).toHaveCount(0);
});

test('old link without a hint: an unsigned kit from a mailbox shows a visible warning', async ({ page }) => {
  await mailbox(page, unsigned());
  await page.goto('/claim#m=mb1&r=readtoken');
  const note = page.locator('#owner-sig');
  await expect(note).toContainText('Nothing shows that the owner made this kit');
  await expect(note).toHaveClass(/warn/);
  await expect(note).not.toHaveClass(/muted/);
});

test('old link without a hint: a signed kit is remembered but not claimed to be the owner\'s', async ({ page }) => {
  await mailbox(page, SIGNED);
  await page.goto('/claim#m=mb1&r=readtoken');
  await expect(page.locator('#owner-sig')).toContainText('does not name the owner');
  await expect(page.locator('#owner-sig')).not.toContainText('Signed by the owner.');
});

test('old link without a hint: another key or plan on the same mailbox warns', async ({ page }) => {
  await mailbox(page, SIGNED);
  await page.goto('/claim#m=mb1&r=readtoken');
  await expect(page.locator('#owner-sig')).toContainText('does not name the owner');
  // The same mailbox now serves a decoy: a kit of a different plan, signed by
  // someone else. The signature must verify, so remember another key first.
  await page.evaluate(() => {
    localStorage.setItem('zandglass.ownermailbox.v1.mb1', `${'0'.repeat(32)}:${'c'.repeat(64)}`);
  });
  await page.reload();
  await expect(page.locator('#owner-sig')).toContainText('different owner key');
});

test('a kit file keeps the current behaviour: no link, no hint', async ({ page }) => {
  await page.goto('/claim');
  await page.setInputFiles('#kit-file', { name: 'kit.zandglass', mimeType: 'application/json', buffer: Buffer.from(SIGNED) });
  await expect(page.locator('#owner-sig')).toContainText('Signed by the owner');
  await page.reload();
  await page.setInputFiles('#kit-file', { name: 'kit.zandglass', mimeType: 'application/json', buffer: Buffer.from(unsigned()) });
  await expect(page.locator('#owner-sig')).toContainText('earlier kits of this plan were');
});

test('notes in the kit carry the same caution as the message', async ({ page }) => {
  await mailbox(page, makeJsonKit());
  await page.goto('/claim#m=mb1&r=readtoken');
  await page.getByLabel('Your 24 words').fill(PHRASE);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'Your inheritance is open' })).toBeVisible();
  await expect(page.locator('#kit-hints')).toContainText(TEST_PAYLOAD.restore_hints[0] ?? '');
  const caution = page.locator('#kit-hints + p');
  await expect(caution).toContainText('treat them with care');
  await expect(caution).toContainText('never send your words or money');
});
