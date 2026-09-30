import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page, type Request } from '@playwright/test';
import { TEST_PAYLOAD, makeJsonKit, read, recordedBeacon } from '../src/test-helpers';

const PHRASE = read('TEST_ONLY_phrase.txt').trim();
const RAW_KIT = read('kit.txt');
const beacon = recordedBeacon();
const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';
const SHOTS = 'test-results/screenshots';
const ALLOWED_HOSTS = new Set(['127.0.0.1:4173', 'api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']);

interface Seen {
  url: string;
  body: string;
  headers: string;
}

/** Record every request, serve drand from the recorded beacon, block all else. */
async function instrument(page: Page, opts: { drandDown?: boolean } = {}): Promise<Seen[]> {
  const seen: Seen[] = [];
  page.on('request', (r: Request) => {
    seen.push({
      url: decodeURIComponent(r.url()),
      body: r.postData() ?? '',
      headers: JSON.stringify(r.headers()),
    });
  });
  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.route(`https://${host}/**`, (route) => {
      if (opts.drandDown) return route.abort('internetdisconnected');
      const url = route.request().url();
      if (url.endsWith(`/${CHAIN}/public/${beacon.round}`)) {
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
  return seen;
}

function wordsIn(text: string): string[] {
  return [...new Set(text.toLowerCase().match(/[a-z]+/g) ?? [])].filter((w) => w.length >= 3);
}

/** No request carries a phrase or seed word, and only allowed hosts were contacted. */
function expectNoLeak(seen: Seen[], secrets: string[]): void {
  expect(seen.length).toBeGreaterThan(0);
  for (const s of seen) {
    const host = new URL(s.url).host;
    if (!s.url.startsWith('data:') && !s.url.startsWith('blob:')) {
      expect(ALLOWED_HOSTS.has(host), `unexpected host ${host}`).toBe(true);
    }
    // Static asset paths of this app (for example "shell-xyz.js") may contain
    // ordinary words, so for same-origin requests only query strings, API paths
    // and bodies are searched. Requests to other hosts are searched in full.
    const u = new URL(s.url);
    const scope = u.host === '127.0.0.1:4173' ? `${u.search}${u.pathname.startsWith('/v1/') ? u.pathname : ''}` : s.url;
    const haystack = `${scope}\n${s.body}`.toLowerCase();
    for (const secret of secrets) {
      for (const w of wordsIn(secret)) {
        expect(new RegExp(`(^|[^a-z])${w}([^a-z]|$)`).test(haystack), `"${w}" leaked in ${s.url}`).toBe(false);
      }
    }
  }
}

async function axe(page: Page): Promise<void> {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    const bad = r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(bad.map((v) => `${scheme}: ${v.id} ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([]);
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function openKitFile(page: Page, kit: string, name = 'kit.sandglass'): Promise<void> {
  await page.setInputFiles('#kit-file', { name, mimeType: 'application/json', buffer: Buffer.from(kit) });
}

test('heir page: create a key, confirm words, show the code', async ({ page }, info) => {
  const seen = await instrument(page);
  await page.goto('/heir.html');
  await expect(page.getByRole('heading', { name: 'Become an heir' })).toBeVisible();
  await axe(page);

  await page.getByRole('button', { name: 'Create my words' }).click();
  const words = await page.locator('.phrase li').allTextContents();
  expect(words).toHaveLength(24);
  await axe(page);
  await page.screenshot({ path: `${SHOTS}/heir-${info.project.name}.png`, fullPage: true });

  await page.getByRole('button', { name: 'I wrote them down' }).click();
  // A wrong word is rejected first.
  const labels = await page.locator('label').allTextContents();
  const positions = labels.map((l) => Number(l.replace('Word ', '')));
  expect(positions).toHaveLength(3);
  for (const p of positions) await page.getByLabel(`Word ${p}`, { exact: true }).fill('wrongword');
  await page.getByRole('button', { name: 'Check my words' }).click();
  await expect(page.getByRole('alert')).toContainText('do not match');
  for (const p of positions) await page.getByLabel(`Word ${p}`, { exact: true }).fill(words[p - 1] ?? '');
  await axe(page);
  await page.getByRole('button', { name: 'Check my words' }).click();

  const code = page.locator('#heir-code');
  await expect(code).toHaveText(/^age1[a-z0-9]{58}$/);
  await expect(page.locator('.qr svg')).toBeVisible();
  await page.screenshot({ path: `${SHOTS}/heir-code-${info.project.name}.png`, fullPage: true });
  await expect(page.getByRole('button', { name: 'Copy heir code' })).toBeVisible();
  await axe(page);

  // Nothing is stored and nothing carries the phrase.
  expect(await page.evaluate(() => localStorage.length + sessionStorage.length)).toBe(0);
  expectNoLeak(seen, [words.join(' ')]);
});

test('claim page: opens the committed vector from a file, without leaking', async ({ page }, info) => {
  const seen = await instrument(page);
  await page.goto('/claim.html');
  await expect(page.getByRole('heading', { name: 'Open your kit' })).toBeVisible();
  await axe(page);

  await openKitFile(page, RAW_KIT, 'vector.txt');
  await expect(page.getByRole('heading', { name: 'This kit is unlocked' })).toBeVisible();
  await expect(page.locator('.facts')).toContainText('32665869');
  await axe(page);

  await page.getByLabel('Your 24 words').fill(PHRASE);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'Your inheritance is open' })).toBeVisible();
  await page.getByRole('button', { name: 'Show the seed' }).click();
  await expect(page.locator('#seed')).toHaveText(read('payload.txt').trim());
  expect(await page.getByLabel('Your 24 words').count()).toBe(0);
  expectNoLeak(seen, [PHRASE]);
});

test('claim page: JSON kit shows message, seed, height and restore steps', async ({ page }, info) => {
  const seen = await instrument(page);
  await page.goto('/claim.html');
  await openKitFile(page, await makeJsonKit());
  await expect(page.locator('.facts')).toContainText('Testnet');
  await page.getByLabel('Your 24 words').fill(PHRASE);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'Your inheritance is open' })).toBeVisible();

  await expect(page.getByText(TEST_PAYLOAD.message)).toBeVisible();
  await expect(page.locator('#seed')).not.toContainText('zoo');
  await page.getByRole('button', { name: 'Show the seed' }).click();
  await expect(page.locator('#seed')).toHaveText(TEST_PAYLOAD.vault_seed);
  await expect(page.getByText('3,500,000')).toBeVisible();
  for (const wallet of ['Zodl', 'Zkool', 'Vizor', 'Zingo', 'Sandglass command line']) {
    await expect(page.getByText(wallet, { exact: true })).toBeVisible();
  }
  await expect(page.getByText('Wallet looks empty?')).toBeVisible();
  await axe(page);
  await page.screenshot({ path: `${SHOTS}/claim-${info.project.name}.png`, fullPage: true });

  expectNoLeak(seen, [PHRASE, TEST_PAYLOAD.vault_seed]);
});

test('claim page: mailbox link fetches the kit with the read token', async ({ page }) => {
  const seen = await instrument(page);
  let auth = '';
  await page.route('**/v1/mailboxes/abc123/kit', (route) => {
    auth = route.request().headers()['authorization'] ?? '';
    return makeJsonKit().then((body) => route.fulfill({ status: 200, contentType: 'application/json', body }));
  });
  await page.goto('/claim.html#m=abc123&r=readtoken');
  await expect(page.getByRole('heading', { name: 'This kit is unlocked' })).toBeVisible();
  expect(auth).toBe('Bearer readtoken');
  await page.getByLabel('Your 24 words').fill(PHRASE);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'Your inheritance is open' })).toBeVisible();
  expectNoLeak(seen, [PHRASE, TEST_PAYLOAD.vault_seed]);
});

test('claim page: states (waiting, wrong key, damaged, network, missing mailbox)', async ({ page }, info) => {
  // Waiting: pretend it is a day before the unlock.
  const unlock = 1692803367 + (beacon.round - 1) * 3;
  await page.clock.setFixedTime(new Date((unlock - 90_000) * 1000));
  await instrument(page);
  await page.goto('/claim.html');
  await openKitFile(page, RAW_KIT);
  await expect(page.getByRole('heading', { name: 'This kit is still locked' })).toBeVisible();
  await expect(page.locator('#countdown')).toHaveText(/^Unlocks in 1d 01h/);
  await expect(page.getByLabel('Your 24 words')).toHaveCount(0);
  await axe(page);
  await page.screenshot({ path: `${SHOTS}/claim-waiting-${info.project.name}.png`, fullPage: true });
});

test('claim page: wrong key, damaged kit, network error and missing mailbox', async ({ page }) => {
  await instrument(page);
  await page.goto('/claim.html');
  await openKitFile(page, RAW_KIT);

  // Not a full phrase.
  await page.getByLabel('Your 24 words').fill('only three words');
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByText('does not look like a full heir phrase')).toBeVisible();

  // A valid phrase that is not the heir's.
  const other = Array(23).fill('abandon').concat('art').join(' ');
  await page.getByLabel('Your 24 words').fill(other);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByText('These words do not open this kit')).toBeVisible();
  await axe(page);

  // Damaged file.
  await page.getByRole('button', { name: 'Open a different kit' }).click();
  await openKitFile(page, '{"format":"sandglass-kit","version":1');
  await expect(page.getByText('This kit looks damaged')).toBeVisible();

  // Missing mailbox.
  await page.route('**/v1/mailboxes/nope/kit', (r) => r.fulfill({ status: 404, body: '' }));
  await page.goto('/claim.html#m=nope&r=x');
  await page.reload();
  await expect(page.getByText('No kit found for this link')).toBeVisible();
});

test('claim page: drand outage shows a retry that works once back', async ({ page }) => {
  await instrument(page, { drandDown: true });
  await page.goto('/claim.html');
  await openKitFile(page, RAW_KIT);
  await page.getByLabel('Your 24 words').fill(PHRASE);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByText('Could not reach the drand time servers')).toBeVisible();
  await axe(page);

  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.unroute(`https://${host}/**`);
    await page.route(`https://${host}/**`, (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: { 'access-control-allow-origin': '*' },
        body: JSON.stringify(beacon),
      }),
    );
  }
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByRole('heading', { name: 'Your inheritance is open' })).toBeVisible();
});

test('landing page links to both tools and passes axe', async ({ page }) => {
  await page.goto('/index.html');
  await expect(page.getByRole('link', { name: 'Create my heir key' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Open a kit' })).toBeVisible();
  await axe(page);
});
