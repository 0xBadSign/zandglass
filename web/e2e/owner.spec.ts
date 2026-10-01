import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page, type Route } from '@playwright/test';

const TOKEN = 'ab'.repeat(24);
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
  await page.getByLabel('Passphrase').fill('correct horse battery');
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
  await page.getByRole('heading', { name: 'New inheritance plan' }).waitFor();
  await page.getByLabel('Heir code 1').fill('age1exampleheir');
  await page.getByLabel('Flip at least every').selectOption('180d');
  await page.getByRole('button', { name: 'Create the plan' }).click();
  await expect(page.locator('#created-title')).toHaveText('Plan 01234567 is ready');
  await expect(page.locator('.created-card .qr-tile svg')).toBeVisible();
  await expect(page.locator('.created-card')).toContainText('https://zandglass.cash/claim#m=abc&r=def');
  expect(body).toMatchObject({ heirs: ['age1exampleheir'], every: '180d', claim_link: true });
});
