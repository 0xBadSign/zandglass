import './style.css';
import './site.css';
import './scenes.css';
import './console.css';
import { clear, copyButton, h, type Child } from './lib/dom';
import { GlassCanvas, mountDust } from './lib/fx';
import { formatCountdown, formatZec } from './lib/playground';
import { qrSvg } from './lib/qr';
import { mountShell } from './lib/shell';

/*
 * The owner panel. It talks only to `zandglass ui` on this computer
 * (same origin, 127.0.0.1) with the session key from the address fragment.
 * Served from the public site it explains how to run it locally instead.
 */

const REPO = 'https://github.com/0xnesQuit/zandglass';
const main = mountShell('owner', { wide: true });
mountDust();

interface Balance {
  total_zat: number;
  spendable_zat: number;
  height: number;
}
interface PlanSum {
  id: string;
  short_id: string;
  kind: string;
  status: 'active' | 'cancelled';
  epoch: number;
  heirs: number;
  period_secs: number;
  created: number;
  unlock_time: number | null;
  epoch_created: number | null;
  mailbox: boolean;
  pending: boolean;
  balance: Balance | null;
  installments: { count: number; amount_zat: number; split: boolean; unlocks: number[] } | null;
}
interface UiState {
  network: 'main' | 'test';
  store: 'missing' | 'locked' | 'open';
  now: number;
  cap_zat: number;
  public_keeper: string;
  plans: PlanSum[];
}
interface Created {
  plan: PlanSum;
  address: string;
  uri: string;
  claim_link: string | null;
}

class ApiErr extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/* ------------------------------------------------------------ session */

const TOKEN_KEY = 'zandglass-owner-session';
function readToken(): string {
  const m = /[#&]t=([0-9a-f]{48})\b/.exec(window.location.hash);
  if (m?.[1]) {
    try {
      sessionStorage.setItem(TOKEN_KEY, m[1]);
    } catch {
      // storage blocked: keep it in memory only
    }
    history.replaceState(null, '', window.location.pathname);
    return m[1];
  }
  try {
    return sessionStorage.getItem(TOKEN_KEY) ?? '';
  } catch {
    return '';
  }
}
const token = readToken();

async function api<T>(method: 'GET' | 'POST', path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: body === undefined ? null : JSON.stringify(body),
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
    });
  } catch {
    throw new ApiErr('network', 'The panel cannot reach `zandglass ui` on this computer. Is it still running?');
  }
  const text = await res.text();
  let json: unknown = null;
  try {
    json = JSON.parse(text);
  } catch {
    json = null;
  }
  if (!res.ok) {
    const e = json as { error?: string; message?: string } | null;
    throw new ApiErr(e?.error ?? `http_${res.status}`, e?.message ?? `The panel answered ${res.status}.`);
  }
  return json as T;
}

/* ------------------------------------------------------------ state */

let state: UiState | null = null;
let skew = 0; // server clock minus this clock, seconds
let selected = '';
let busy = '';
let created: Created | null = null;
let showNew = false;
let fundFor = '';
let fund: { address: string; uri: string } | null = null;
let cancelFor = '';
let problem: { tone: 'bad' | 'warn' | 'ok' | 'info'; title: string; body: string } | null = null;
let mode: 'local' | 'remote' | 'loading' = token ? 'loading' : 'remote';

const nowSec = (): number => Date.now() / 1000 + skew;
const unit = (): string => (state?.network === 'test' ? 'TAZ' : 'ZEC');
const zec = (zat: number): string => formatZec(zat, unit());

function setState(s: UiState): void {
  state = s;
  skew = s.now - Date.now() / 1000;
  const active = s.plans.filter((p) => p.status === 'active');
  if (!s.plans.some((p) => p.id === selected)) selected = active[0]?.id ?? s.plans[0]?.id ?? '';
}

async function act(label: string, fn: () => Promise<void>): Promise<void> {
  if (busy) return;
  busy = label;
  problem = null;
  render();
  try {
    await fn();
  } catch (e) {
    const err = e instanceof ApiErr ? e : new ApiErr('error', String(e));
    problem = { tone: err.code === 'network' ? 'warn' : 'bad', title: titleFor(err.code), body: err.message };
  } finally {
    busy = '';
    render();
  }
}

function titleFor(code: string): string {
  const t: Record<string, string> = {
    unauthorized: 'Not allowed',
    network: 'Connection problem',
    insufficient_funds: 'Not enough funds',
    invalid_request: 'Check the form',
    heir_code_invalid: 'An heir code is not valid',
  };
  return t[code] ?? 'Something went wrong';
}

function download(name: string, text: string, type: string): void {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = h('a', { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 2000);
}

/* ------------------------------------------------------------ stage */

const glass = new GlassCanvas({ className: 'pg-glass' });
glass.set(0);
glass.setFlowing(false);
const stageNet = h('span', { class: 'badge' }, 'Owner');
const stageTime = h('p', { class: 'countdown', role: 'timer' });
const stageCaption = h('p', { class: 'stage-caption' });
const vaultRow = h('ol', { class: 'vault-row', 'aria-label': 'Vault epochs' });
const stage = h(
  'aside',
  { class: 'app-stage', 'aria-label': 'Selected plan' },
  h('div', { class: 'stage-top' }, h('span', { class: 'stage-k' }, 'Your vault'), stageNet),
  h('div', { class: 'stage-glass' }, h('div', { class: 'halo', 'aria-hidden': 'true' }), glass.el),
  stageTime,
  stageCaption,
  h('div', { class: 'stage-vaults' }, h('span', { class: 'stage-k' }, 'Vault epochs'), vaultRow),
);

function cd(label: string, time: string): void {
  stageTime.replaceChildren(h('span', { class: 'cd-label' }, label), h('span', { class: 'cd-time' }, time));
}

function paintStage(): void {
  if (state) {
    stageNet.textContent = state.network === 'test' ? 'Testnet' : 'Mainnet';
    stageNet.className = state.network === 'test' ? 'badge test' : 'badge';
  }
  const p = state?.plans.find((x) => x.id === selected);
  if (!p || p.status !== 'active' || !p.unlock_time) {
    glass.set(0);
    glass.setFlowing(false);
    cd('Unlocks in', '--d --h');
    stageCaption.textContent =
      mode === 'remote'
        ? 'The owner panel runs on your own computer.'
        : state?.store === 'open'
          ? 'Create a plan to set the sand running.'
          : 'Unlock your store to see your plans.';
    vaultRow.replaceChildren(h('li', { class: 'none' }, 'No vault yet'));
    return;
  }
  const start = p.epoch_created ?? p.unlock_time - p.period_secs;
  const total = Math.max(1, p.unlock_time - start);
  const left = p.unlock_time - nowSec();
  glass.set(Math.min(1, Math.max(0, 1 - left / total)));
  glass.setFlowing(left > 0);
  if (left > 0) cd(`Plan ${p.short_id} unlocks in`, formatCountdown(left));
  else cd(`Plan ${p.short_id}`, 'Unlocked');
  stageCaption.textContent =
    left > 0 ? 'Flip before the sand runs out to keep the funds yours.' : 'The sand has run out: your heirs can open the kit now. Flip at once if the funds are still there.';
  const n = Math.max(1, p.epoch);
  const shown = Math.min(n, 6);
  vaultRow.replaceChildren(
    ...Array.from({ length: shown }, (_, i) => {
      const e = n - shown + i + 1;
      const cur = e === n;
      return h(
        'li',
        { class: cur ? 'cur' : 'old' },
        h('span', { class: 'v-n' }, `#${e}`),
        h('span', { class: 'v-s' }, cur ? 'funds' : 'empty'),
      );
    }),
  );
}

window.setInterval(() => {
  paintStage();
  document.querySelectorAll<HTMLElement>('[data-unlock]').forEach((el) => {
    const left = Number(el.dataset.unlock) - nowSec();
    el.textContent = left > 0 ? formatCountdown(left) : 'Unlocked';
    el.classList.toggle('late', left <= 0);
  });
}, 1000);

/* ------------------------------------------------------------ views */

function periodText(s: number): string {
  if (s % 86400 === 0) return s === 86400 ? '1 day' : `${s / 86400} days`;
  if (s % 3600 === 0) return s === 3600 ? '1 hour' : `${s / 3600} hours`;
  if (s % 60 === 0) return s === 60 ? '1 minute' : `${s / 60} minutes`;
  return `${s} seconds`;
}

function notice(tone: string, title: string, body: Child): HTMLElement {
  return h('div', { class: `notice ${tone}`, role: tone === 'bad' ? 'alert' : 'status' }, h('div', {}, h('strong', {}, title), h('p', {}, body)));
}

function passForm(kind: 'init' | 'unlock'): HTMLElement {
  const p1 = h('input', { type: 'password', id: 'pass1', autocomplete: kind === 'init' ? 'new-password' : 'current-password' });
  const p2 = h('input', { type: 'password', id: 'pass2', autocomplete: 'new-password' });
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, kind === 'init' ? 'Create your store' : 'Unlock your store'),
    h(
      'p',
      {},
      kind === 'init'
        ? 'The store keeps your vault seeds on this computer, encrypted with a passphrase. If you lose the passphrase you lose access to the vaults, but your heirs are not affected: their kits hold the seeds too.'
        : 'Your vault seeds are encrypted on this computer. The passphrase stays here; it is never sent anywhere else.',
    ),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          if (kind === 'init' && p1.value !== p2.value) {
            err.textContent = 'The two passphrases do not match.';
            return;
          }
          const pass = p1.value;
          void act(kind === 'init' ? 'Creating the store...' : 'Unlocking...', async () => {
            setState(await api<UiState>('POST', kind === 'init' ? '/api/init' : '/api/unlock', { passphrase: pass }));
          });
        },
      },
      h('label', { for: 'pass1' }, 'Passphrase'),
      p1,
      kind === 'init' ? h('label', { for: 'pass2' }, 'Repeat the passphrase') : null,
      kind === 'init' ? p2 : null,
      err,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', disabled: busy ? true : null }, kind === 'init' ? 'Create the store' : 'Unlock')),
    ),
  );
}

function planCard(p: PlanSum): HTMLElement {
  const active = p.status === 'active';
  const sel = p.id === selected;
  const head = h(
    'div',
    { class: 'plan-head' },
    h('span', { class: 'plan-id' }, p.kind === 'installments' ? 'Installments' : 'Legacy', ' · ', h('code', {}, p.short_id)),
    h('span', { class: active ? 'badge' : 'badge test' }, active ? 'Active' : 'Cancelled'),
  );
  const stats = h(
    'dl',
    { class: 'plan-stats' },
    h('div', {}, h('dt', {}, 'Unlocks in'), h('dd', { 'data-unlock': String(p.unlock_time ?? 0), class: 'plan-left' }, p.unlock_time ? formatCountdown(p.unlock_time - nowSec()) : '-')),
    h('div', {}, h('dt', {}, 'Balance'), h('dd', {}, p.balance ? zec(p.balance.total_zat) : 'Press Sync', p.balance && p.balance.spendable_zat < p.balance.total_zat ? h('span', { class: 'confirming' }, 'confirming') : null)),
    h('div', {}, h('dt', {}, 'Flip every'), h('dd', {}, periodText(p.period_secs))),
    h('div', {}, h('dt', {}, 'Heirs · vault'), h('dd', {}, `${p.heirs} · #${p.epoch}`)),
  );
  const parts: Child[] = [head, stats];
  if (p.pending) parts.push(notice('warn', 'A flip is not settled yet', 'Press Sync: it checks whether the last flip reached the network.'));
  if (active && p.balance && state?.network === 'main' && p.balance.total_zat > state.cap_zat) {
    parts.push(notice('warn', 'Above the safety cap', `This plan holds more than ${zec(state.cap_zat)}. Zandglass is unaudited; a flip refuses above the cap unless the panel was started with --accept-unaudited.`));
  }
  const confirming = p.balance !== null && p.balance.spendable_zat < p.balance.total_zat;
  if (active && confirming) {
    parts.push(notice('info', 'Funds are confirming', 'New funds can be flipped once they have a few confirmations on the network (about 10 to 15 minutes). Press Sync to check.'));
  }
  if (active) {
    const flipBtn = h(
      'button',
      {
        type: 'button',
        class: 'btn-primary',
        disabled: busy || confirming ? true : null,
        onclick: () =>
          void act(`Flipping plan ${p.short_id}: moving the funds to a new vault...`, async () => {
            const r = await api<{ plan: PlanSum; moved_zat: number; txids: string[]; mailbox_error: string | null }>('POST', `/api/plans/${p.id}/flip`);
            void glass.flip();
            setState(await api<UiState>('GET', '/api/state'));
            problem = r.mailbox_error
              ? { tone: 'warn', title: 'Flipped, but the claim link still serves the old kit', body: `The funds moved (${zec(r.moved_zat)}), but the keeper could not be updated: ${r.mailbox_error}. Download the new kit and give it to your heirs.` }
              : { tone: 'ok', title: `Flipped plan ${p.short_id}`, body: `Moved ${zec(r.moved_zat)} into vault #${r.plan.epoch}. The timer started again; every older kit now opens an empty vault. Transaction ${r.txids.join(', ')}` };
          }),
      },
      "Flip (I'm still here)",
    );
    const fundBtn = h(
      'button',
      {
        type: 'button',
        class: 'btn',
        disabled: busy ? true : null,
        onclick: () =>
          void act('Loading the funding address...', async () => {
            if (fundFor === p.id) {
              fundFor = '';
              fund = null;
              return;
            }
            fund = await api<{ address: string; uri: string }>('GET', `/api/plans/${p.id}/receive`);
            fundFor = p.id;
          }),
      },
      fundFor === p.id ? 'Hide funding QR' : 'Add funds',
    );
    const kitBtn =
      p.kind === 'installments'
        ? null
        : h(
            'button',
            {
              type: 'button',
              class: 'btn',
              disabled: busy ? true : null,
              onclick: () =>
                void act('Reading the kit...', async () => {
                  const k = await api<{ file_name: string; text: string }>('GET', `/api/plans/${p.id}/kit`);
                  download(k.file_name, k.text, 'application/json');
                }),
            },
            'Download kit',
          );
    const calBtn = h(
      'button',
      {
        type: 'button',
        class: 'btn',
        disabled: busy ? true : null,
        onclick: () =>
          void act('Making the calendar file...', async () => {
            const c = await api<{ file_name: string; text: string }>('GET', `/api/plans/${p.id}/calendar`);
            download(c.file_name, c.text, 'text/calendar');
          }),
      },
      'Calendar reminder',
    );
    const cancelBtn = h(
      'button',
      { type: 'button', class: 'btn-quiet', disabled: busy ? true : null, onclick: () => ((cancelFor = cancelFor === p.id ? '' : p.id), render()) },
      'Cancel plan',
    );
    parts.push(h('div', { class: 'actions' }, flipBtn, fundBtn, kitBtn, calBtn, cancelBtn));
    if (fundFor === p.id && fund) {
      const f = fund;
      parts.push(
        h(
          'div',
          { class: 'fund-box' },
          h('div', { class: 'qr-tile' }, qrSvg(f.uri, 'QR code of the funding request')),
          h(
            'div',
            {},
            h('p', {}, 'Scan with any Zcash wallet (Zashi, Zodl, Zingo, Ywallet...) and send from a shielded balance. The funds land in this plan’s current vault.'),
            h('code', { class: 'code-box' }, f.address),
            h('div', { class: 'actions' }, copyButton('Copy address', () => f.address), copyButton('Copy payment request', () => f.uri)),
            h('p', { class: 'muted small' }, 'Then press Sync to see the balance.'),
          ),
        ),
      );
    }
    if (cancelFor === p.id) {
      const to = h('input', { type: 'text', id: `to-${p.short_id}`, placeholder: state?.network === 'test' ? 'utest1...' : 'u1...', autocomplete: 'off', spellcheck: 'false' });
      parts.push(
        h(
          'form',
          {
            class: 'cancel-box',
            novalidate: true,
            onsubmit: (e: Event) => {
              e.preventDefault();
              const addr = to.value.trim();
              void act(`Cancelling plan ${p.short_id}...`, async () => {
                const r = await api<{ moved_zat: number; txids: string[] }>('POST', `/api/plans/${p.id}/cancel`, { to: addr });
                cancelFor = '';
                setState(await api<UiState>('GET', '/api/state'));
                problem = { tone: 'ok', title: `Plan ${p.short_id} cancelled`, body: `Moved ${zec(r.moved_zat)} to your address. Old kits now open empty vaults. Transaction ${r.txids.join(', ')}` };
              });
            },
          },
          h('label', { for: `to-${p.short_id}` }, 'Send everything back to this shielded address'),
          to,
          h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn', disabled: busy ? true : null }, 'Cancel and withdraw')),
        ),
      );
    }
  }
  return h(
    'article',
    {
      class: `plan-card${sel ? ' sel' : ''}${active ? '' : ' off'}`,
      onclick: (e: Event) => {
        if ((e.target as HTMLElement).closest('button, input, a, form')) return;
        selected = p.id;
        render();
      },
    },
    ...parts,
  );
}

const PERIODS: [string, string][] = [
  ['30d', '30 days'],
  ['90d', '90 days'],
  ['180d', '6 months'],
  ['365d', '1 year'],
];

function newPlanForm(): HTMLElement {
  const heirs = [0, 1, 2].map((i) =>
    h('input', { type: 'text', id: `heir${i}`, placeholder: 'age1...', autocomplete: 'off', spellcheck: 'false', 'aria-label': `Heir code ${i + 1}` }),
  );
  const every = h('select', { id: 'every' }, ...PERIODS.map(([v, t]) => h('option', { value: v, selected: v === '90d' ? true : null }, t)), h('option', { value: '10m' }, '10 minutes (try it out)'));
  const message = h('textarea', { id: 'message', rows: '3', placeholder: 'Optional. Sealed into the kit; only your heirs can read it, after the unlock.' });
  const link = h('input', { type: 'checkbox', id: 'claimlink', checked: true });
  const amount = h('input', { type: 'text', id: 'amount', inputmode: 'decimal', placeholder: 'Optional, for example 0.1' });
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card', id: 'new-plan' },
    h('h2', { tabindex: '-1' }, 'New inheritance plan'),
    h('p', {}, 'Your heirs first make a key on the ', h('a', { href: 'https://zandglass.cash/heir', target: '_blank', rel: 'noreferrer noopener' }, 'Become an heir page'), ' and send you their public heir code.'),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          const codes = heirs.map((x) => x.value.trim()).filter(Boolean);
          if (!codes.length) {
            err.textContent = 'Add at least one heir code.';
            return;
          }
          void act('Creating a fresh vault and sealing the kit...', async () => {
            created = await api<Created>('POST', '/api/plans', {
              heirs: codes,
              every: every.value,
              message: message.value,
              claim_link: link.checked,
              amount: amount.value.trim() || null,
            });
            showNew = false;
            setState(await api<UiState>('GET', '/api/state'));
            selected = created.plan.id;
          });
        },
      },
      h('fieldset', { class: 'heir-fields' }, h('legend', {}, 'Heir codes (one is enough; up to three, any of them can claim)'), ...heirs),
      h('label', { for: 'every' }, 'Flip at least every'),
      every,
      h('label', { for: 'amount' }, 'Amount to request in the funding QR (ZEC)'),
      amount,
      h('label', { for: 'message' }, 'A message for your heirs'),
      message,
      h('label', { class: 'check' }, link, h('span', {}, 'Keep the kit in a blind mailbox at ', h('strong', {}, 'zandglass.cash'), ' so your heir gets a claim link that always serves the newest kit. The keeper stores only ciphertext.')),
      err,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', disabled: busy ? true : null }, 'Create the plan')),
    ),
  );
}

function createdCard(c: Created): HTMLElement {
  return h(
    'section',
    { class: 'card created-card' },
    h('h2', { tabindex: '-1', id: 'created-title' }, `Plan ${c.plan.short_id} is ready`),
    h('p', {}, '1. Fund the vault: scan the QR code with any Zcash wallet.'),
    h(
      'div',
      { class: 'fund-box' },
      h('div', { class: 'qr-tile' }, qrSvg(c.uri, 'QR code of the funding request')),
      h('div', {}, h('code', { class: 'code-box' }, c.address), h('div', { class: 'actions' }, copyButton('Copy address', () => c.address), copyButton('Copy payment request', () => c.uri))),
    ),
    c.claim_link
      ? h(
          'div',
          {},
          h('p', {}, '2. Send this claim link to your heir now. It is shown only once:'),
          h('code', { class: 'code-box' }, c.claim_link),
          h('div', { class: 'actions' }, copyButton('Copy claim link', () => c.claim_link ?? '', 'btn-primary')),
          h('p', { class: 'muted small' }, 'The link opens nothing by itself: your heir also needs their own 24 words, and must wait for the unlock time.'),
        )
      : h('p', {}, '2. Download the kit file below and give it to your heirs.'),
    h('p', {}, '3. Add the flip reminders to your calendar, and flip before each unlock.'),
    h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn', onclick: () => ((created = null), render()) }, 'Done')),
  );
}

function remoteView(): HTMLElement {
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, 'The owner panel runs on your computer'),
    h('p', {}, 'Your vault seeds never leave your own machine, so the owner side of Zandglass is not a website. Install the command line tool and start the panel; it opens this page with a private session key.'),
    h('ol', { class: 'checklist' }, h('li', {}, 'Install Rust (rustup.rs), then build Zandglass: ', h('code', {}, `cargo install --git ${REPO} zandglass-cli`)), h('li', {}, 'Start the panel: ', h('code', {}, 'zandglass ui'), ' (add ', h('code', {}, '--network test'), ' to try it with test coins).'), h('li', {}, 'Create a plan, scan the QR code with your wallet to fund it, and send the claim link to your heir.')),
    h('p', { class: 'muted' }, 'Prefer a terminal? Every action here is also a command: ', h('code', {}, 'zandglass plan legacy'), ', ', h('code', {}, 'flip'), ', ', h('code', {}, 'status'), ', ', h('code', {}, 'remind'), '.'),
    h('div', { class: 'actions' }, h('a', { class: 'btn-primary', href: './playground' }, 'Try it in the playground first'), h('a', { class: 'btn', href: REPO }, 'Source code')),
  );
}

function render(): void {
  clear(main);
  const flow = h(
    'div',
    { class: 'app-flow' },
    h('span', { class: 'eyebrow' }, 'On your computer only'),
    h('h1', {}, 'Owner ', h('em', {}, 'panel')),
    h('p', { class: 'lead' }, 'Create a plan, fund it, flip now and then. Your seeds stay on this machine.'),
  );
  main.append(h('div', { class: 'app-console owner-console' }, stage, flow));
  paintStage();
  if (busy) flow.append(h('p', { class: 'busy-line', role: 'status' }, h('span', { class: 'spinner' }), ` ${busy}`));
  if (problem) flow.append(notice(problem.tone, problem.title, problem.body));
  if (mode === 'remote') {
    flow.append(remoteView());
    return;
  }
  if (mode === 'loading' || !state) {
    flow.append(h('div', { class: 'card', role: 'status' }, h('span', { class: 'spinner' }), ' Connecting to zandglass ui...'));
    return;
  }
  if (state.store === 'missing') {
    flow.append(passForm('init'));
    return;
  }
  if (state.store === 'locked') {
    flow.append(passForm('unlock'));
    return;
  }
  flow.append(
    h(
      'div',
      { class: 'owner-bar' },
      h('span', { class: state.network === 'test' ? 'badge test' : 'badge' }, state.network === 'test' ? 'Testnet (test coins)' : 'Mainnet (real ZEC)'),
      h(
        'button',
        {
          type: 'button',
          class: 'btn',
          disabled: busy ? true : null,
          onclick: () =>
            void act('Syncing with the network...', async () => {
              setState(await api<UiState>('POST', '/api/sync'));
            }),
        },
        'Sync',
      ),
      h('button', { type: 'button', class: 'btn', disabled: busy ? true : null, onclick: () => ((showNew = !showNew), render()) }, showNew ? 'Close the form' : 'New plan'),
      h(
        'button',
        {
          type: 'button',
          class: 'btn-quiet',
          disabled: busy ? true : null,
          onclick: () =>
            void act('Locking...', async () => {
              setState(await api<UiState>('POST', '/api/lock'));
            }),
        },
        'Lock',
      ),
    ),
  );
  if (created) flow.append(createdCard(created));
  if (showNew || state.plans.length === 0) flow.append(newPlanForm());
  if (state.plans.length) flow.append(h('h2', { class: 'plans-title' }, 'Your plans'), ...state.plans.map(planCard));
}

/* ------------------------------------------------------------ start */

render();
if (token) {
  void api<UiState>('GET', '/api/state')
    .then((s) => {
      mode = 'local';
      setState(s);
      render();
    })
    .catch((e: unknown) => {
      mode = e instanceof ApiErr && e.code === 'unauthorized' ? 'local' : 'remote';
      if (mode === 'local') {
        state = null;
        problem = { tone: 'bad', title: 'This session key is not valid', body: 'Open the address that `zandglass ui` printed in your terminal (it changes every time the panel starts).' };
      }
      render();
    });
}
