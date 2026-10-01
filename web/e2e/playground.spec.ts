import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page, type Request } from '@playwright/test';
import { Encrypter } from 'age-encryption';
import { Buffer, timelockEncrypt } from 'tlock-js';
import { QUICKNET_INFO } from '../src/lib/drand';
import { TEST_PAYLOAD, recordedBeacon, recordedClient } from '../src/test-helpers';

const SHOTS = 'test-results/screenshots';
const JUDGE = 'JUDGE-TEST-CODE';
const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';
const beacon = recordedBeacon();
const ORIGIN = 'http://127.0.0.1:4173';
const ALLOWED_HOSTS = new Set(['127.0.0.1:4173', 'api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']);
const TXID = (c: string) => c.repeat(64);

async function axe(page: Page): Promise<void> {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21aa']).analyze();
    const bad = r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical');
    expect(bad.map((v) => `${scheme}: ${v.id} ${v.nodes.map((n) => n.target).join(' ')}`)).toEqual([]);
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

/** A kit sealed to `heirCode` and to the recorded drand round, like the keeper would make. */
async function kitFor(heirCode: string): Promise<string> {
  const enc = new Encrypter();
  enc.addRecipient(heirCode);
  const inner = await enc.encrypt(JSON.stringify({ ...TEST_PAYLOAD, network: 'main' }));
  const sealed = await timelockEncrypt(beacon.round, Buffer.from(inner), recordedClient());
  return JSON.stringify({
    format: 'sandglass-kit',
    version: 1,
    kind: 'legacy',
    network: 'main',
    created: '2026-10-01T00:00:00Z',
    plan: '0123456789abcdef0123456789abcdef',
    epoch: 1,
    unlock: { drand_chain: QUICKNET_INFO.hash, round: beacon.round, time: '2026-10-01T00:00:00Z' },
    sealed,
  });
}

interface Mock {
  /** Let the timer run out on the next poll. */
  unlock: boolean;
  createBody: Record<string, unknown> | null;
  settleBody: Record<string, unknown> | null;
  mailboxAuth: string;
  createError: { status: number; code: string } | null;
  statusEnabled: boolean;
  state: string;
}

interface Seen {
  url: string;
  body: string;
  headers: string;
}

/** Mock the keeper (session lifecycle, mailbox) and drand; record every request. */
async function setup(page: Page, opts: Partial<Mock> = {}): Promise<{ mock: Mock; seen: Seen[] }> {
  const mock: Mock = {
    unlock: false,
    createBody: null,
    settleBody: null,
    mailboxAuth: '',
    createError: null,
    statusEnabled: true,
    state: 'funding',
    ...opts,
  };
  const seen: Seen[] = [];
  page.on('request', (r: Request) => {
    seen.push({ url: decodeURIComponent(r.url()), body: r.postData() ?? '', headers: JSON.stringify(r.headers()) });
  });
  for (const host of ['api.drand.sh', 'api2.drand.sh', 'drand.cloudflare.com']) {
    await page.route(`https://${host}/**`, (route) => {
      const ok = route.request().url().endsWith(`/${CHAIN}/public/${beacon.round}`);
      return route.fulfill({
        status: ok ? 200 : 404,
        contentType: 'application/json',
        headers: { 'access-control-allow-origin': '*' },
        body: ok ? JSON.stringify(beacon) : '',
      });
    });
  }

  let polls = 0;
  let unlockAt = Date.now() + 150_000;
  let flips = 0;
  let minutes = 3;
  let heirCode = '';
  let finalState = '';
  const txs: { kind: string; txid: string; explorer: string }[] = [];
  const tx = (kind: string, c: string) => ({ kind, txid: TXID(c), explorer: `https://explorer.example/tx/${TXID(c)}` });
  const json = (status: number, body: unknown) => ({
    status,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
  const snapshot = () => ({
    id: 'sess1',
    state: mock.state,
    epoch: flips + 1,
    unlock_time: new Date(unlockAt).toISOString(),
    balance_zat: 20000,
    confirmed: polls >= 3,
    txs,
    claim_link: `${ORIGIN}/claim#m=mbx1&r=readtok`,
    message: `mock state ${mock.state}`,
    error: null,
  });

  await page.route('**/v1/playground/status', (route) =>
    route.fulfill(
      json(200, {
        enabled: mock.statusEnabled,
        network: 'main',
        amount_zat: 20000,
        active: 2,
        max_active: 10,
        today: 7,
        max_per_day: 60,
        float_ready: true,
      }),
    ),
  );
  await page.route('**/v1/playground/sessions', async (route) => {
    const body = JSON.parse(route.request().postData() ?? '{}') as Record<string, unknown>;
    mock.createBody = body;
    if (mock.createError) {
      return route.fulfill(json(mock.createError.status, { error: mock.createError.code, message: 'server text' }));
    }
    if (body['judge_code'] !== JUDGE) {
      return route.fulfill(json(403, { error: 'judge_code_invalid', message: 'nope' }));
    }
    heirCode = String(body['heir_code']);
    minutes = Number(body['minutes']);
    unlockAt = Date.now() + minutes * 60_000 - 30_000;
    return route.fulfill(
      json(201, {
        id: 'sess1',
        token: 'sess-token',
        claim_link: `${ORIGIN}/claim#m=mbx1&r=readtok`,
        network: 'main',
        amount_zat: 20000,
        minutes,
      }),
    );
  });
  await page.route('**/v1/playground/sessions/sess1', (route) => {
    expect(route.request().headers()['authorization']).toBe('Bearer sess-token');
    polls++;
    if (mock.state === 'funding' && polls >= 2) {
      mock.state = 'sealed';
      txs.push(tx('fund', 'a'));
    }
    if (mock.state === 'sealed' && mock.unlock) {
      unlockAt = Date.now() - 1000;
      mock.state = 'unlocked';
    }
    if (mock.state === 'settling') {
      mock.state = finalState;
      txs.push(tx(finalState === 'sent' ? 'send' : 'return', 'c'));
    }
    return route.fulfill(json(200, snapshot()));
  });
  await page.route('**/v1/playground/sessions/sess1/flip', (route) => {
    flips++;
    unlockAt = Date.now() + minutes * 60_000;
    txs.push(tx('flip', 'b'));
    return route.fulfill(json(200, snapshot()));
  });
  await page.route('**/v1/playground/sessions/sess1/settle', (route) => {
    const body = JSON.parse(route.request().postData() ?? '{}') as Record<string, unknown>;
    mock.settleBody = body;
    finalState = body['action'] === 'send' ? 'sent' : body['action'] === 'keep' ? 'kept' : 'returned';
    if (finalState === 'kept') {
      mock.state = 'kept';
    } else {
      mock.state = 'settling';
    }
    return route.fulfill(json(200, snapshot()));
  });
  await page.route('**/v1/mailboxes/mbx1/kit', async (route) => {
    mock.mailboxAuth = route.request().headers()['authorization'] ?? '';
    return route.fulfill({ status: 200, contentType: 'application/json', body: await kitFor(heirCode) });
  });
  return { mock, seen };
}

/** No request carries the heir phrase or the vault seed; only allowed hosts were contacted. */
function expectNoLeak(seen: Seen[], mock: Mock, secrets: string[]): void {
  expect(seen.length).toBeGreaterThan(0);
  for (const s of seen) {
    expect(ALLOWED_HOSTS.has(new URL(s.url).host), `unexpected host in ${s.url}`).toBe(true);
    const hay = `${s.url}\n${s.body}\n${s.headers}`.toLowerCase();
    const flat = ` ${hay.replace(/[^a-z]+/g, ' ')} `;
    expect(hay).not.toContain('age-secret-key');
    for (const secret of secrets) {
      const words = secret.toLowerCase().split(/\s+/);
      for (let i = 0; i + 3 <= words.length; i++) {
        expect(flat.includes(` ${words.slice(i, i + 3).join(' ')} `), `secret words leaked in ${s.url}`).toBe(false);
      }
    }
  }
  // The only body the page sends is the public heir code, the timer and the judge code.
  expect(Object.keys(mock.createBody ?? {}).sort()).toEqual(['heir_code', 'judge_code', 'minutes']);
  expect(String(mock.createBody?.['heir_code'])).toMatch(/^age1[a-z0-9]{58}$/);
  if (mock.settleBody) expect(Object.keys(mock.settleBody).every((k) => k === 'action' || k === 'to')).toBe(true);
}

async function toHeir(page: Page): Promise<void> {
  await page.goto('/playground?poll=300');
  await expect(page.getByRole('heading', { name: '1. Enter the judge code' })).toBeVisible();
  await page.getByLabel('Judge code').fill(JUDGE);
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('heading', { name: '2. Become the heir' })).toBeVisible();
}

async function toVault(page: Page, minutes = 3): Promise<string> {
  await toHeir(page);
  await page.getByRole('button', { name: 'Create my heir key' }).click();
  await expect(page.locator('.phrase li')).toHaveCount(24);
  const words = (await page.locator('.phrase li').allTextContents()).join(' ');
  await page.getByRole('button', { name: 'Use this heir code' }).click();
  await expect(page.getByRole('heading', { name: '3. Start a vault' })).toBeVisible();
  await page.getByLabel(`${minutes} minutes`).check();
  await page.getByRole('button', { name: 'Start the vault' }).click();
  await expect(page.getByRole('heading', { name: '3. Your vault is running' })).toBeVisible();
  return words;
}

async function toOpened(page: Page, mock: Mock): Promise<string> {
  const words = await toVault(page);
  mock.unlock = true;
  await expect(page.getByRole('heading', { name: '4. Open the kit' })).toBeVisible();
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'The kit is open' })).toBeVisible();
  return words;
}

test('playground: full judge flow with a flip, in-browser open and return', async ({ page }, info) => {
  const { mock, seen } = await setup(page);
  await page.goto('/playground?poll=300');

  // Step 1: the label says what this is, the code is only in sessionStorage.
  await expect(page.locator('.demo-banner')).toContainText('Mainnet demo');
  await expect(page.locator('.demo-banner')).toContainText('Your heir key never leaves this browser');
  await expect(page.locator('#status-line')).toContainText('0.0002 ZEC');
  await axe(page);
  await page.getByLabel('Judge code').fill(JUDGE);
  await page.getByRole('button', { name: 'Continue' }).click();

  // Step 2: the heir key is made in the browser.
  await expect(page.getByRole('heading', { name: '2. Become the heir' })).toBeVisible();
  await page.getByRole('button', { name: 'Create my heir key' }).click();
  await expect(page.locator('.phrase li')).toHaveCount(24);
  await expect(page.locator('#heir-code')).toHaveText(/^age1[a-z0-9]{58}$/);
  await expect(page.getByText('Demo shortcut')).toBeVisible();
  await axe(page);
  const words = (await page.locator('.phrase li').allTextContents()).join(' ');
  await page.getByRole('button', { name: 'Use this heir code' }).click();

  // Step 3: start a 3 minute vault.
  await expect(page.getByRole('heading', { name: '3. Start a vault' })).toBeVisible();
  await axe(page);
  await page.getByLabel('3 minutes').check();
  await page.getByRole('button', { name: 'Start the vault' }).click();
  await expect(page.getByRole('heading', { name: '3. Your vault is running' })).toBeVisible();
  expect(mock.createBody?.['minutes']).toBe(3);

  const tl = page.locator('.timeline li');
  await expect(tl).toHaveCount(5);
  await expect(page.locator('li[data-step="funded"]')).toHaveClass(/done/);
  await expect(page.locator('li[data-step="funded"] a')).toHaveAttribute('href', `https://explorer.example/tx/${TXID('a')}`);
  await expect(page.locator('li[data-step="sealed"]')).toHaveClass(/done/);
  await expect(page.locator('#countdown')).toHaveText(/^Unlocks in 0[12]m \d\ds$/);
  // Flip is allowed once the funding is confirmed.
  await expect(page.getByRole('button', { name: "Flip (I'm still here)" })).toBeEnabled();
  await axe(page);
  await page.screenshot({ path: `${SHOTS}/playground-countdown-${info.project.name}.png`, fullPage: true });

  await page.getByRole('button', { name: "Flip (I'm still here)" }).click();
  await expect(page.locator('li[data-step="flip"]')).toHaveClass(/done/);
  await expect(page.locator('li[data-step="flip"]')).toContainText('Flipped 1 time');

  await page.getByRole('button', { name: 'Stop flipping' }).click();
  await expect(page.getByRole('button', { name: "Flip (I'm still here)" })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Resume flipping' })).toBeVisible();
  await expect(page.locator('#flip-hint')).toContainText('stopped flipping');

  // Step 4: the sand runs out, the kit opens in the browser.
  mock.unlock = true;
  await expect(page.getByRole('heading', { name: '4. Open the kit' })).toBeVisible();
  await expect(page.locator('li[data-step="sand"]')).toHaveClass(/done/);
  await page.getByRole('button', { name: 'Open the kit' }).click();
  await expect(page.getByRole('heading', { name: 'The kit is open' })).toBeVisible();
  expect(mock.mailboxAuth).toBe('Bearer readtok');
  await expect(page.getByText(TEST_PAYLOAD.message)).toBeVisible();
  await expect(page.locator('#seed')).not.toContainText('zoo');
  await expect(page.locator('#birthday')).toHaveText('3,500,000');
  await page.getByRole('button', { name: 'Show the seed' }).click();
  await expect(page.locator('#seed')).toHaveText(TEST_PAYLOAD.vault_seed);
  await page.getByRole('button', { name: 'Hide the seed' }).click();
  await expect(page.locator('#seed')).not.toContainText('zoo');
  await axe(page);
  await page.screenshot({ path: `${SHOTS}/playground-opened-${info.project.name}.png`, fullPage: true });

  // Settle: the default is to return the coins.
  await expect(page.getByLabel('Return the coins to the demo')).toBeChecked();
  await page.getByRole('button', { name: 'Confirm' }).click();
  await expect(page.locator('#outcome')).toContainText('The coins went back to the demo wallet');
  await expect(page.getByText(TXID('c'))).toBeVisible();
  await expect(page.locator('li[data-step="open"]')).toHaveClass(/done/);
  expect(mock.settleBody).toEqual({ action: 'return' });
  await axe(page);

  // The judge code is only in sessionStorage; the heir words nowhere.
  expect(await page.evaluate(() => Object.keys(sessionStorage))).toEqual(['sandglass-judge-code']);
  expect(await page.evaluate(() => localStorage.length)).toBe(0);
  expectNoLeak(seen, mock, [words, TEST_PAYLOAD.vault_seed]);
});

test('playground: send to my address validates the address, then sends', async ({ page }) => {
  const { mock, seen } = await setup(page);
  const words = await toOpened(page, mock);
  await page.getByLabel('Send to my address').check();
  await page.getByLabel('Your unified address').fill('t1notunified');
  await page.getByRole('button', { name: 'Confirm' }).click();
  await expect(page.getByText('Enter a unified address for mainnet')).toBeVisible();
  expect(mock.settleBody).toBeNull();

  const ua = `u1${'q'.repeat(70)}`;
  await page.getByLabel('Your unified address').fill(ua);
  await page.getByRole('button', { name: 'Confirm' }).click();
  await expect(page.locator('#outcome')).toContainText('sent to your address');
  expect(mock.settleBody).toEqual({ action: 'send', to: ua });
  await expect(page.locator('.txlist')).toContainText('send');
  expectNoLeak(seen, mock, [words, TEST_PAYLOAD.vault_seed]);
});

test('playground: keep it shows restore guidance', async ({ page }) => {
  const { mock } = await setup(page);
  await toOpened(page, mock);
  await page.getByLabel('Keep it (restore in your own wallet)').check();
  await page.getByRole('button', { name: 'Confirm' }).click();
  await expect(page.locator('#outcome')).toContainText('The vault is yours');
  expect(mock.settleBody).toEqual({ action: 'keep' });
  await expect(page.locator('#birthday')).toHaveText('3,500,000');
  await expect(page.getByRole('button', { name: 'Try again with a new heir key' })).toBeVisible();
});

test('playground: error codes are shown in plain English', async ({ page }) => {
  const { mock } = await setup(page);
  await page.goto('/playground?poll=300');
  await page.getByLabel('Judge code').fill('WRONG');
  await page.getByRole('button', { name: 'Continue' }).click();
  await page.getByRole('button', { name: 'Create my heir key' }).click();
  await page.getByRole('button', { name: 'Use this heir code' }).click();
  await page.getByRole('button', { name: 'Start the vault' }).click();
  // A wrong judge code sends the visitor back to step 1.
  await expect(page.getByRole('heading', { name: '1. Enter the judge code' })).toBeVisible();
  await expect(page.getByRole('alert')).toContainText('That judge code is not accepted');

  // The sessionStorage code is remembered, so only the heir step needs a click.
  await page.getByLabel('Judge code').fill(JUDGE);
  await page.getByRole('button', { name: 'Continue' }).click();
  await page.getByRole('button', { name: 'Use this heir code' }).click();
  const cases: [number, string, string][] = [
    [503, 'playground_closed', 'The playground is closed right now'],
    [429, 'playground_busy', 'The playground is busy'],
    [429, 'daily_cap_reached', 'limit for today'],
    [500, 'weird_server_code', 'server had a problem'],
  ];
  for (const [status, code, text] of cases) {
    mock.createError = { status, code };
    await page.getByRole('button', { name: 'Start the vault' }).click();
    await expect(page.locator('#problems')).toContainText(text);
    await expect(page.getByRole('heading', { name: '3. Start a vault' })).toBeVisible();
  }
});

test('playground: closed playground and unreachable server are explained', async ({ page }) => {
  await setup(page, { statusEnabled: false });
  await page.goto('/playground');
  await expect(page.getByText('The playground is closed right now')).toBeVisible();
  await axe(page);

  const other = await page.context().newPage();
  await other.route('**/v1/playground/status', (r) => r.abort('connectionrefused'));
  await other.goto('/playground');
  await expect(other.getByText('Cannot reach the playground server')).toBeVisible();
});

test('playground: a session that disappears is reported', async ({ page }) => {
  const { mock } = await setup(page);
  await toVault(page);
  await page.unroute('**/v1/playground/sessions/sess1');
  await page.route('**/v1/playground/sessions/sess1', (r) =>
    r.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: 'session_not_found', message: 'x' }) }),
  );
  await expect(page.locator('#problems')).toContainText('This session was not found');
  expect(mock.createBody).not.toBeNull();
});
