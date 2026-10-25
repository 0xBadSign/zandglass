import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page, type Route } from '@playwright/test';
import { read } from '../src/test-helpers';

const TOKEN = 'ab'.repeat(24);
// A real 24-word test phrase and the heir code it derives (committed test vectors).
const WORDS = read('TEST_ONLY_phrase.txt').trim();
const WORDS_CODE = read('heir_code.txt').trim();
const now = Math.floor(Date.now() / 1000);

const plan = (over: Record<string, unknown> = {}) => ({
  id: '0123456789abcdef0123456789abcdef',
  short_id: '01234567',
  kind: 'legacy',
  status: 'active',
  epoch: 1,
  heirs: 1,
  period_secs: 90 * 86400,
  created: now - 3600,
  unlock_time: now + 89 * 86400,
  epoch_created: now - 86400,
  mailbox: true,
  pending: false,
  balance: { total_zat: 10_000_000, spendable_zat: 10_000_000, height: 3_500_000 },
  installments: null,
  ...over,
});

const state = (over: Record<string, unknown> = {}) => ({
  network: 'main',
  store: 'open',
  now,
  cap_zat: 25_000_000,
  public_keeper: 'https://zandglass.cash',
  plans: [plan()],
  ...over,
});

async function mockApi(page: Page, handlers: Record<string, (r: Route) => unknown>): Promise<string[]> {
  const calls: string[] = [];
  await page.route('**/api/**', async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname;
    calls.push(`${req.method()} ${path}`);
    expect(req.headers()['authorization']).toBe(`Bearer ${TOKEN}`);
    const h = handlers[`${req.method()} ${path}`];
    if (h) return h(route);
    return route.fulfill({ status: 404, contentType: 'application/json', body: '{"error":"not_found","message":"no"}' });
  });
  return calls;
}

const json = (body: unknown, status = 200) => (r: Route) =>
  r.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

test('owner: on the public site it explains how to run the panel locally', async ({ page }) => {
  await page.goto('/owner');
  await expect(page.getByRole('heading', { name: 'The owner panel runs on your computer' })).toBeVisible();
  await expect(page.getByText('zandglass ui', { exact: true }).first()).toBeVisible();
  const r = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(r.violations.filter((v) => v.impact === 'serious' || v.impact === 'critical').map((v) => v.id)).toEqual([]);
});

test('owner: unlock, see a plan, flip, download the calendar file', async ({ page }) => {
  let unlocked = false;
  const calls = await mockApi(page, {
    'GET /api/state': (r) => json(unlocked ? state() : state({ store: 'locked', plans: [] }))(r),
    'POST /api/unlock': (r) => {
      expect(JSON.parse(r.request().postData() ?? '{}')).toEqual({ passphrase: 'correct horse battery' });
      unlocked = true;
      return json(state())(r);
    },
    'POST /api/plans/0123456789abcdef0123456789abcdef/flip': json({
      plan: plan({ epoch: 2 }),
      moved_zat: 9_990_000,
      adopted: false,
      txids: ['cd'.repeat(32)],
      mailbox_error: null,
    }),
    'GET /api/plans/0123456789abcdef0123456789abcdef/calendar': json({ file_name: 'zandglass-01234567.ics', text: 'BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n' }),
  });
  await page.goto(`/owner#t=${TOKEN}`);
  // The session key leaves the address bar at once.
  await expect(page).toHaveURL(/\/owner$/);
  await page.getByLabel('Your passphrase').fill('correct horse battery');
  await page.getByRole('button', { name: 'Unlock' }).click();
  const card = page.locator('.plan-card');
  await expect(card).toContainText('01234567');
  await expect(card).toContainText('0.1 ZEC');
  await expect(card).toContainText('90 days');

  await page.getByRole('button', { name: "Flip (I'm still here)" }).click();
  await expect(page.locator('.notice.ok')).toContainText('Flipped plan 01234567');

  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Calendar reminder' }).click()]);
  expect(dl.suggestedFilename()).toBe('zandglass-01234567.ics');
  expect(calls).toContain('POST /api/plans/0123456789abcdef0123456789abcdef/flip');
});

test('owner: unconfirmed funds disable the flip; errors are explained', async ({ page }) => {
  await mockApi(page, {
    'GET /api/state': json(state({ plans: [plan({ balance: { total_zat: 5_000_000, spendable_zat: 0, height: 1 } })] })),
    'POST /api/sync': json({ error: 'error', message: 'no lightwalletd server could be reached' }, 500),
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await expect(page.getByText('Funds are confirming')).toBeVisible();
  await expect(page.getByRole('button', { name: "Flip (I'm still here)" })).toBeDisabled();
  await page.getByRole('button', { name: 'Sync' }).click();
  await expect(page.getByRole('alert')).toContainText('no lightwalletd server could be reached');
});

test('owner: create a plan and show the funding QR and the claim link once', async ({ page }) => {
  let body: Record<string, unknown> = {};
  await mockApi(page, {
    'GET /api/state': json(state({ plans: [] })),
    'POST /api/plans': (r) => {
      body = JSON.parse(r.request().postData() ?? '{}') as Record<string, unknown>;
      return json({
        plan: plan(),
        address: 'u1exampleaddress',
        uri: 'zcash:u1exampleaddress?amount=0.1',
        claim_link: 'https://zandglass.cash/claim#m=abc&r=def',
      })(r);
    },
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('heading', { name: 'New plan', exact: true }).waitFor();
  await page.getByLabel('Heir code 1').fill('age1exampleheir');
  await expect(page.locator('#code0-fp')).toContainText(/Code ID [0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}/);
  await page.getByLabel('Flip at least every').selectOption('180d');
  await page.getByRole('button', { name: 'Create the plan' }).click();
  await expect(page.locator('#created-title')).toHaveText('Plan 01234567 is ready');
  await expect(page.locator('.created-card .qr-tile svg')).toBeVisible();
  await expect(page.locator('.created-card')).toContainText('https://zandglass.cash/claim#m=abc&r=def');
  expect(body).toMatchObject({ heirs: ['age1exampleheir'], every: '180d', claim_link: true });
});

test('owner: create a scheduled-payments plan and start the schedule', async ({ page }) => {
  let body: Record<string, unknown> = {};
  let split = false;
  const instPlan = (s: boolean) =>
    plan({
      kind: 'installments',
      period_secs: 30 * 86400,
      installments: { count: 3, amount_zat: 5_000_000, split: s, unlocks: s ? [now + 30 * 86400, now + 60 * 86400, now + 90 * 86400] : [] },
    });
  await mockApi(page, {
    'GET /api/state': (r) => json(state({ plans: [instPlan(split)] }))(r),
    'POST /api/installments': (r) => {
      body = JSON.parse(r.request().postData() ?? '{}') as Record<string, unknown>;
      return json({ plan: instPlan(false), address: 'u1fundingaddress', uri: 'zcash:u1fundingaddress?amount=0.1503', required_zat: 15_030_000, claim_link: null })(r);
    },
    'POST /api/plans/0123456789abcdef0123456789abcdef/split': (r) => {
      split = true;
      return json({
        plan: instPlan(true),
        txid: 'ef'.repeat(32),
        lines: [1, 2, 3].map((i) => ({ index: i, unlock_time: now + i * 30 * 86400, kit_file: `01234567-i${i}of3.zandglass` })),
        links_file: '01234567-claim-links.txt',
        links: 'installment 1: https://zandglass.cash/claim#m=a&r=b\n',
      })(r);
    },
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('button', { name: 'New plan' }).click();
  await page.getByRole('button', { name: /Scheduled payments/ }).click();
  await page.getByLabel('Payee code 1').fill('age1examplepayee');
  await page.getByLabel(/Each installment/).fill('0.05');
  await page.getByLabel('Number of installments').fill('3');
  await expect(page.getByText(/The payee receives 3 × 0.05 ZEC/)).toBeVisible();
  await page.getByRole('button', { name: 'Create the schedule' }).click();
  await expect(page.locator('.created-card')).toContainText('Fund the plan with 0.1503 ZEC');
  expect(body).toMatchObject({ payees: ['age1examplepayee'], amount: '0.05', every: '30d', count: 3, claim_link: true });
  await page.getByRole('button', { name: 'Done' }).click();

  await page.getByRole('button', { name: 'Start the schedule' }).click();
  await expect(page.locator('#split-title')).toContainText('has started');
  await expect(page.locator('.inst-list li')).toHaveCount(3);
  const [dl] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: 'Download the claim links' }).click()]);
  expect(dl.suggestedFilename()).toBe('01234567-claim-links.txt');
  await expect(page.locator('.plan-card')).toContainText('3 × 0.05 ZEC');
});

test('owner: a custom flip period is sent as a duration', async ({ page }) => {
  let body: Record<string, unknown> = {};
  await mockApi(page, {
    'GET /api/state': json(state({ plans: [] })),
    'POST /api/plans': (r) => {
      body = JSON.parse(r.request().postData() ?? '{}') as Record<string, unknown>;
      return json({ plan: plan(), address: 'u1x', uri: 'zcash:u1x', claim_link: null })(r);
    },
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByLabel('Heir code 1').fill('age1exampleheir');
  await expect(page.getByLabel('Custom amount of time')).toBeHidden();
  await page.getByLabel('Flip at least every').selectOption('custom');
  await page.getByLabel('Custom amount of time').fill('2');
  await page.getByLabel('Unit').selectOption('mo');
  await page.getByRole('button', { name: 'Create the plan' }).click();
  await expect(page.locator('#created-title')).toBeVisible();
  expect(body).toMatchObject({ every: '60d' });
});

const LOCK_ID = '0123456789abcdef0123456789abcdef';
const lockPlan = () =>
  plan({
    kind: 'lock',
    period_secs: 180 * 86400,
    unlock_time: now + 180 * 86400,
    epoch_created: now,
    mailbox: false,
    balance: null,
  });

test('owner: a savings lock needs the box and the word LOCK, then hides flip and cancel', async ({ page }) => {
  let body: Record<string, unknown> | null = null;
  let made = false;
  const calls = await mockApi(page, {
    'GET /api/state': (r) => json(state({ plans: made ? [lockPlan()] : [] }))(r),
    'POST /api/locks': (r) => {
      body = JSON.parse(r.request().postData() ?? '{}') as Record<string, unknown>;
      made = true;
      return json({ plan: lockPlan(), address: 'u1lockaddress', uri: 'zcash:u1lockaddress', claim_link: null })(r);
    },
    [`GET /api/plans/${LOCK_ID}/kit`]: json({ file_name: '01234567-e1.zandglass', text: '{}' }),
  });
  // Every request body and URL the page sends, to prove the words never leave the tab.
  const sent: string[] = [];
  page.on('request', (r) => {
    if (r.url().includes('/api/')) sent.push(`${r.url()}\n${r.postData() ?? ''}`);
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('button', { name: /Savings lock/ }).click();
  await expect(page.getByText('A savings lock cannot be undone')).toBeVisible();
  // No heir code can be pasted any more: the form asks for the words.
  await expect(page.getByLabel('Your own heir code')).toHaveCount(0);
  // A phrase that is not a valid 24-word phrase is refused before anything is sent.
  await page.getByLabel('Your 24 heir words').fill('abandon abandon abandon');
  await page.getByLabel('Locked until').selectOption('180d');
  await page.getByLabel(/I have my 24 heir words/).check();
  await page.getByLabel('Type LOCK to confirm').fill('LOCK');
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('.field-error')).toContainText('24 heir words');
  expect(calls).not.toContain('POST /api/locks');
  await page.getByLabel(/I have my 24 heir words/).uncheck();
  await page.getByLabel('Type LOCK to confirm').fill('');
  await page.getByLabel('Your 24 heir words').fill(WORDS);
  // The page shows the derived code's short id as soon as the words are valid.
  await expect(page.locator('#lock-derived')).toContainText(/heir code ID [0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}/);
  await page.getByLabel('Locked until').selectOption('180d');
  await expect(page.getByText(/The lock ends on/)).toBeVisible();

  // Nothing is sent until the box is ticked and LOCK is typed.
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('.field-error')).toContainText('Tick the box');
  await page.getByLabel(/I have my 24 heir words/).check();
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('.field-error')).toContainText('Type the word LOCK');
  await page.getByLabel('Type LOCK to confirm').fill('lock');
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('.field-error')).toContainText('Type the word LOCK');
  expect(calls).not.toContain('POST /api/locks');

  await page.getByLabel('Type LOCK to confirm').fill('LOCK');
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('#created-title')).toHaveText('Plan 01234567 is ready');
  await expect(page.locator('.created-card .qr-tile svg')).toBeVisible();
  await expect(page.locator('.created-card')).toContainText('seed was deleted');
  expect(body).toMatchObject({ heirs: [WORDS_CODE], until: '180d', claim_link: false, understood: true, confirm: 'LOCK' });
  // Only the derived public code was sent: not one of the 24 words appears in any request.
  const everything = sent.join('\n').toLowerCase();
  expect(everything).toContain(WORDS_CODE);
  for (const word of new Set(WORDS.split(/\s+/))) {
    expect(everything, `the word "${word}" must not be sent`).not.toMatch(new RegExp(`\\b${word}\\b`));
  }
  expect(everything).not.toContain(WORDS.toLowerCase());
  await page.getByRole('button', { name: 'Done' }).click();

  const card = page.locator('.plan-card');
  await expect(card).toContainText('Savings lock');
  await expect(card).toContainText('Locked until');
  await expect(card).toContainText('seed deleted');
  await expect(card.getByRole('button', { name: /Flip/ })).toHaveCount(0);
  await expect(card.getByRole('button', { name: /Cancel/ })).toHaveCount(0);
  const [dl] = await Promise.all([page.waitForEvent('download'), card.getByRole('button', { name: 'Download kit' }).click()]);
  expect(dl.suggestedFilename()).toBe('01234567-e1.zandglass');
});

test('owner: a savings lock can end on a picked date', async ({ page }) => {
  let body: Record<string, unknown> = {};
  await mockApi(page, {
    'GET /api/state': json(state({ plans: [] })),
    'POST /api/locks': (r) => {
      body = JSON.parse(r.request().postData() ?? '{}') as Record<string, unknown>;
      return json({ plan: lockPlan(), address: 'u1lockaddress', uri: 'zcash:u1lockaddress', claim_link: null })(r);
    },
  });
  await page.goto(`/owner#t=${TOKEN}`);
  await page.getByRole('button', { name: /Savings lock/ }).click();
  await page.getByLabel('Your 24 heir words').fill(WORDS);
  await expect(page.getByLabel('Unlock date')).toBeHidden();
  await page.getByLabel('Locked until').selectOption('date');
  await page.getByLabel('Unlock date').fill('2099-01-15');
  await page.getByLabel(/I have my 24 heir words/).check();
  await page.getByLabel('Type LOCK to confirm').fill('LOCK');
  await page.getByRole('button', { name: 'Create the savings lock' }).click();
  await expect(page.locator('#created-title')).toBeVisible();
  expect(String(body.until)).toMatch(/^2099-01-1[45]T\d{2}:\d{2}:\d{2}Z$/);
});
