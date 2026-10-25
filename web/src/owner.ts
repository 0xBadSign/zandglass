import './style.css';
import './site.css';
import './scenes.css';
import './console.css';
import { clear, copyButton, h, type Child } from './lib/dom';
import { GlassCanvas, mountDust } from './lib/fx';
import { formatCountdown, formatZec } from './lib/playground';
import { dateToUntil, untilEpoch } from './lib/lock';
import { MAX_PARTS, PART_ECC, kitQrText, splitKit } from './lib/paper';
import { qrSvg } from './lib/qr';
import { mountShell } from './lib/shell';

/*
 * The owner panel. It talks only to `zandglass ui` on this computer
 * (same origin, 127.0.0.1) with the session key from the address fragment.
 * Served from the public site it explains how to run it locally instead.
 */

const REPO = 'https://github.com/0xBadSign/zandglass';
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
  network_fixed?: boolean;
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
  required_zat?: number;
}
interface SplitDone {
  plan: PlanSum;
  txid: string;
  lines: { index: number; unlock_time: number; kit_file: string }[];
  links_file: string | null;
  links: string | null;
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
let newKind: 'legacy' | 'installments' | 'lock' = 'legacy';
let splitDone: SplitDone | null = null;
let fundFor = '';
let fund: { address: string; uri: string } | null = null;
let paperFor = '';
let paperEpoch = 0;
let paper: { parts: string[]; fits: boolean } | null = null;

function makePaper(text: string): { parts: string[]; fits: boolean } {
  const parts = splitKit(text);
  return { parts, fits: parts.length <= MAX_PARTS };
}
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

/** The next unlock of a plan: the kit for legacy plans, the next installment otherwise. */
function nextUnlock(p: PlanSum): { at: number; from: number; label: string } | null {
  if (p.kind === 'installments') {
    const s = p.installments;
    if (!s || !s.split || !s.unlocks.length) return null;
    const now = nowSec();
    const i = s.unlocks.findIndex((u) => u > now);
    const k = i < 0 ? s.unlocks.length - 1 : i;
    const at = s.unlocks[k] ?? 0;
    const from = k > 0 ? (s.unlocks[k - 1] ?? at - p.period_secs) : at - p.period_secs;
    return { at, from, label: `Installment ${k + 1} of ${s.count}` };
  }
  if (!p.unlock_time) return null;
  return { at: p.unlock_time, from: p.epoch_created ?? p.unlock_time - p.period_secs, label: p.kind === 'lock' ? 'Savings lock' : `Plan ${p.short_id}` };
}

/** A unix time as a readable local date and time. */
function whenText(unix: number): string {
  return new Date(unix * 1000).toLocaleString(undefined, { dateStyle: 'long', timeStyle: 'short' });
}

function paintStage(): void {
  if (state) {
    stageNet.textContent = state.network === 'test' ? 'Testnet' : 'Mainnet';
    stageNet.className = state.network === 'test' ? 'badge test' : 'badge';
  }
  const p = state?.plans.find((x) => x.id === selected);
  const next = p && p.status === 'active' ? nextUnlock(p) : null;
  if (!p || !next) {
    glass.set(0);
    glass.setFlowing(false);
    cd('Unlocks in', '--d --h');
    stageCaption.textContent =
      mode === 'remote'
        ? 'The owner panel runs on your own computer.'
        : p?.kind === 'installments' && p.status === 'active'
          ? 'Fund the plan, then start the schedule.'
          : state?.store === 'open'
            ? 'Create a plan to set the sand running.'
            : 'Unlock your store to see your plans.';
    vaultRow.replaceChildren(h('li', { class: 'none' }, 'No vault yet'));
    return;
  }
  const left = next.at - nowSec();
  glass.set(Math.min(1, Math.max(0, 1 - left / Math.max(1, next.at - next.from))));
  glass.setFlowing(left > 0);
  cd(left > 0 ? `${next.label} unlocks in` : next.label, left > 0 ? formatCountdown(left) : 'Unlocked');
  if (p.kind === 'installments' && p.installments) {
    stageCaption.textContent = left > 0 ? 'Each installment opens for the payee on its date. Cancel takes back what has not opened.' : 'Every installment is open for the payee.';
    const now = nowSec();
    const all = p.installments.unlocks;
    const nextIdx = all.findIndex((x) => x > now);
    const items: HTMLElement[] = all
      .slice(0, 8)
      .map((u, i) =>
        h('li', { class: u <= now ? 'old' : i === nextIdx ? 'cur' : '' }, h('span', { class: 'v-n' }, `#${i + 1}`), h('span', { class: 'v-s' }, u <= now ? 'open' : 'sealed')),
      );
    if (all.length > 8) items.push(h('li', { class: 'none' }, `+${all.length - 8} more`));
    vaultRow.replaceChildren(...items);
    return;
  }
  if (p.kind === 'lock') {
    stageCaption.textContent =
      left > 0
        ? 'Locked from yourself: nobody, you included, can move these funds before the sand runs out.'
        : 'The lock has opened. Open the kit with your heir words on the claim page.';
    vaultRow.replaceChildren(h('li', { class: left > 0 ? 'cur' : 'old' }, h('span', { class: 'v-n' }, '#1'), h('span', { class: 'v-s' }, left > 0 ? 'locked' : 'open')));
    return;
  }
  stageCaption.textContent =
    left > 0 ? 'Flip before the sand runs out to keep the funds yours.' : 'The sand has run out: your heirs can open the kit now. Flip at once if the funds are still there.';
  const n = Math.max(1, p.epoch);
  const shown = Math.min(n, 6);
  vaultRow.replaceChildren(
    ...Array.from({ length: shown }, (_, i) => {
      const e = n - shown + i + 1;
      const cur = e === n;
      return h('li', { class: cur ? 'cur' : 'old' }, h('span', { class: 'v-n' }, `#${e}`), h('span', { class: 'v-s' }, cur ? 'funds' : 'empty'));
    }),
  );
}

window.setInterval(() => {
  paintStage();
  document.querySelectorAll<HTMLElement>('[data-unlock]').forEach((el) => {
    if (!Number(el.dataset.unlock)) return;
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

/** Mainnet or testnet. Each network has its own store; switching locks the current one. */
function networkSwitch(): HTMLElement | null {
  if (!state || state.network_fixed) return null;
  const opt = (n: 'main' | 'test', label: string) =>
    h(
      'button',
      {
        type: 'button',
        class: `net${state?.network === n ? ' on' : ''}`,
        'aria-pressed': String(state?.network === n),
        disabled: busy || state?.network === n ? true : null,
        onclick: () =>
          void act(`Switching to ${label}...`, async () => {
            setState(await api<UiState>('POST', '/api/network', { network: n }));
            created = null;
            splitDone = null;
          }),
      },
      label,
    );
  return h('div', { class: 'net-switch', role: 'group', 'aria-label': 'Network' }, opt('main', 'Mainnet'), opt('test', 'Testnet (test coins)'));
}

function passForm(kind: 'init' | 'unlock'): HTMLElement {
  const p1 = h('input', { type: 'password', id: 'pass1', autocomplete: kind === 'init' ? 'new-password' : 'current-password' });
  const p2 = h('input', { type: 'password', id: 'pass2', autocomplete: 'new-password' });
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, kind === 'init' ? 'Create your store' : 'Unlock your store'),
    networkSwitch(),
    h(
      'p',
      {},
      kind === 'init'
        ? 'First time here: choose a new passphrase. Nobody gives it to you; you make it up now and type it again whenever you open the panel. It encrypts your vault seeds on this computer. If you lose it you lose access to the vaults, but your heirs are not affected: their kits hold the seeds too.'
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
      h('label', { for: 'pass1' }, kind === 'init' ? 'Choose a new passphrase (at least 10 characters)' : 'Your passphrase'),
      p1,
      kind === 'init' ? h('label', { for: 'pass2' }, 'Repeat the passphrase') : null,
      kind === 'init' ? p2 : null,
      err,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', disabled: busy ? true : null }, kind === 'init' ? 'Create the store' : 'Unlock')),
    ),
  );
}

/**
 * The paper kit: one QR code of the current kit, the short plan id and the
 * unlock date. No amounts, no addresses. A copy goes into #paper-print, the
 * only thing the print stylesheet shows.
 */
function paperSheet(p: PlanSum, k: { parts: string[] }): HTMLElement {
  const when = p.unlock_time ? new Date(p.unlock_time * 1000).toLocaleDateString(undefined, { dateStyle: 'long' }) : 'unknown';
  const n = k.parts.length;
  return h(
    'div',
    { class: 'paper-sheet' },
    h('h2', {}, 'Zandglass paper kit'),
    h('p', { class: 'paper-meta' }, 'Plan ', h('code', {}, p.short_id), ' · unlocks ', h('strong', {}, when)),
    h(
      'div',
      { class: 'paper-grid' },
      ...k.parts.map((t, i) =>
        h(
          'figure',
          { class: 'paper-part' },
          h('div', { class: 'paper-qr' }, qrSvg(t, `QR code, part ${i + 1} of ${n}, of the kit of plan ${p.short_id}`, PART_ECC)),
          h('figcaption', {}, n > 1 ? `Part ${i + 1} of ${n}` : 'Kit'),
        ),
      ),
    ),
    h(
      'p',
      {},
      n > 1
        ? `Scan every part on zandglass.cash/claim (any order). You also need the heir phrase, which is not on this page.`
        : 'Scan on zandglass.cash/claim. You also need the heir phrase, which is not on this page.',
    ),
    h('p', { class: 'paper-small' }, p.kind === 'lock' ? 'This kit never changes: a savings lock is not flipped.' : 'This kit is replaced if the owner flips the plan: always use the newest one.'),
  );
}

function printPaper(p: PlanSum, k: { parts: string[] }): void {
  document.getElementById('paper-print')?.remove();
  const root = h('div', { id: 'paper-print' }, paperSheet(p, k));
  document.body.append(root);
  window.addEventListener('afterprint', () => root.remove(), { once: true });
  window.print();
}

function paperView(p: PlanSum, k: { parts: string[]; fits: boolean }): HTMLElement {
  if (!k.fits) {
    return h(
      'div',
      { class: 'notice warn', id: 'paper-kit' },
      h(
        'div',
        {},
        h('strong', {}, 'This kit is too big for paper'),
        h('p', {}, 'It has too much data (many heirs or a long message) to print as a few QR codes. Use "Download kit" and give your heirs the file instead.'),
      ),
    );
  }
  return h(
    'div',
    { class: 'paper-box', id: 'paper-kit' },
    paperSheet(p, k),
    h(
      'div',
      { class: 'actions' },
      h('button', { type: 'button', class: 'btn-primary', onclick: () => printPaper(p, k) }, 'Print'),
    ),
    h('p', { class: 'muted small' }, 'The kit is public ciphertext. Heirs also need their heir phrase. Print a new one after every flip.'),
  );
}

function fundButton(p: PlanSum): HTMLElement {
  return h(
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
}

function kitButton(p: PlanSum): HTMLElement {
  return h(
    'button',
    {
      type: 'button',
      class: 'btn-primary',
      disabled: busy ? true : null,
      onclick: () =>
        void act('Reading the kit...', async () => {
          const k = await api<{ file_name: string; text: string }>('GET', `/api/plans/${p.id}/kit`);
          download(k.file_name, k.text, 'application/json');
        }),
    },
    'Download kit',
  );
}

function paperButton(p: PlanSum): HTMLElement {
  return h(
    'button',
    {
      type: 'button',
      class: 'btn',
      disabled: busy ? true : null,
      onclick: () =>
        void act('Reading the kit...', async () => {
          if (paperFor === p.id) {
            paperFor = '';
            paper = null;
            return;
          }
          const k = await api<{ file_name: string; text: string }>('GET', `/api/plans/${p.id}/kit`);
          const text = kitQrText(k.text);
          paper = makePaper(text);
          paperFor = p.id;
          paperEpoch = p.epoch;
        }),
    },
    paperFor === p.id ? 'Hide paper kit' : 'Print paper kit',
  );
}

function calendarButton(p: PlanSum, label: string): HTMLElement {
  return h(
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
    label,
  );
}

/** The four numbers on a savings-lock card. There is no balance: the seed (and with it the wallet) is gone. */
function lockStats(p: PlanSum, next: { at: number } | null): HTMLElement {
  return h(
    'dl',
    { class: 'plan-stats' },
    h('div', {}, h('dt', {}, 'Unlocks in'), h('dd', { 'data-unlock': String(next?.at ?? 0), class: 'plan-left' }, next ? formatCountdown(next.at - nowSec()) : '-')),
    h('div', {}, h('dt', {}, 'Unlocks on'), h('dd', {}, p.unlock_time ? new Date(p.unlock_time * 1000).toLocaleDateString(undefined, { dateStyle: 'medium' }) : '-')),
    h('div', {}, h('dt', {}, 'Seed'), h('dd', {}, 'Deleted')),
    h('div', {}, h('dt', {}, 'Sealed to'), h('dd', {}, `${p.heirs} heir key${p.heirs === 1 ? '' : 's'}`)),
  );
}

function planCard(p: PlanSum): HTMLElement {
  const active = p.status === 'active';
  const sel = p.id === selected;
  const lock = p.kind === 'lock';
  const head = h(
    'div',
    { class: 'plan-head' },
    h('span', { class: 'plan-id' }, p.kind === 'installments' ? 'Installments' : lock ? 'Savings lock' : 'Legacy', ' · ', h('code', {}, p.short_id)),
    h('span', { class: active ? 'badge' : 'badge test' }, active ? 'Active' : 'Cancelled'),
  );
  const inst = p.kind === 'installments' ? p.installments : null;
  const next = active ? nextUnlock(p) : null;
  const stats = lock ? lockStats(p, next) : h(
    'dl',
    { class: 'plan-stats' },
    h(
      'div',
      {},
      h('dt', {}, inst ? 'Next installment' : 'Unlocks in'),
      h('dd', { 'data-unlock': String(next?.at ?? 0), class: 'plan-left' }, next ? formatCountdown(next.at - nowSec()) : inst && !inst.split ? 'Not started' : '-'),
    ),
    h('div', {}, h('dt', {}, inst?.split ? 'Left in funding' : 'Balance'), h('dd', {}, p.balance ? zec(p.balance.total_zat) : 'Press Sync', p.balance && p.balance.spendable_zat < p.balance.total_zat ? h('span', { class: 'confirming' }, 'confirming') : null)),
    inst
      ? h('div', {}, h('dt', {}, 'Schedule'), h('dd', {}, `${inst.count} × ${zec(inst.amount_zat)}`))
      : h('div', {}, h('dt', {}, 'Flip every'), h('dd', {}, periodText(p.period_secs))),
    inst
      ? h('div', {}, h('dt', {}, 'Every'), h('dd', {}, periodText(p.period_secs)))
      : h('div', {}, h('dt', {}, 'Heirs · vault'), h('dd', {}, `${p.heirs} · #${p.epoch}`)),
  );
  const parts: Child[] = [head, stats];
  if (lock && p.unlock_time) {
    parts.push(
      h('p', { class: 'lock-line' }, h('strong', {}, `Locked until ${whenText(p.unlock_time)} · seed deleted`)),
      h(
        'p',
        { class: 'muted small' },
        'Nobody can move these funds before then, not even you, so there is nothing to flip or cancel. After the unlock, open the kit with your heir words on zandglass.cash/claim or with `zandglass claim`.',
      ),
    );
  }
  if (p.pending) parts.push(notice('warn', 'A flip is not settled yet', 'Press Sync: it checks whether the last flip reached the network.'));
  if (active && p.balance && state?.network === 'main' && p.balance.total_zat > state.cap_zat) {
    parts.push(notice('warn', 'Above the safety cap', `This plan holds more than ${zec(state.cap_zat)}. Zandglass is unaudited; a flip refuses above the cap unless the panel was started with --accept-unaudited.`));
  }
  const confirming = p.balance !== null && p.balance.spendable_zat < p.balance.total_zat;
  if (active && confirming && !(inst && inst.split)) {
    parts.push(
      notice(
        'info',
        'Funds are confirming',
        inst
          ? 'You can start the schedule once the funds have a few confirmations on the network (about 10 to 15 minutes). Press Sync to check.'
          : 'New funds can be flipped once they have a few confirmations on the network (about 10 to 15 minutes). Press Sync to check.',
      ),
    );
  }
  if (active && inst) {
    const startBtn = inst.split
      ? null
      : h(
          'button',
          {
            type: 'button',
            class: 'btn-primary',
            disabled: busy || confirming ? true : null,
            onclick: () =>
              void act(`Starting the schedule of plan ${p.short_id}: one transaction into ${inst.count} installment vaults...`, async () => {
                splitDone = await api<SplitDone>('POST', `/api/plans/${p.id}/split`);
                setState(await api<UiState>('GET', '/api/state'));
              }),
          },
          'Start the schedule',
        );
    const fundBtn = inst.split
      ? null
      : h(
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
    const kitsBtn = inst.split
      ? h(
          'button',
          {
            type: 'button',
            class: 'btn',
            disabled: busy ? true : null,
            onclick: () =>
              void act('Reading the kits...', async () => {
                const k = await api<{ files: { file_name: string; text: string }[] }>('GET', `/api/plans/${p.id}/kits`);
                for (const f of k.files) download(f.file_name, f.text, f.file_name.endsWith('.txt') ? 'text/plain' : 'application/json');
              }),
          },
          p.mailbox ? 'Download claim links and kits' : 'Download kits',
        )
      : null;
    const calBtn = inst.split
      ? h(
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
          'Calendar',
        )
      : null;
    const cancelBtn = h('button', { type: 'button', class: 'btn-quiet', disabled: busy ? true : null, onclick: () => ((cancelFor = cancelFor === p.id ? '' : p.id), render()) }, 'Cancel plan');
    parts.push(h('div', { class: 'actions' }, startBtn, fundBtn, kitsBtn, calBtn, cancelBtn));
    if (!inst.split) parts.push(h('p', { class: 'muted small' }, 'Fund the plan with the amount in the QR request. When the funds have confirmed, start the schedule: one ordinary transaction fills one vault per installment and seals one kit for each.'));
  }
  if (active && lock) {
    parts.push(h('div', { class: 'actions' }, fundButton(p), kitButton(p), paperButton(p), calendarButton(p, 'Calendar reminder')));
  }
  if (active && !inst && !lock) {
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
    const paperBtn =
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
                  if (paperFor === p.id) {
                    paperFor = '';
                    paper = null;
                    return;
                  }
                  const k = await api<{ file_name: string; text: string }>('GET', `/api/plans/${p.id}/kit`);
                  const text = kitQrText(k.text);
                  paper = makePaper(text);
                  paperFor = p.id;
                  paperEpoch = p.epoch;
                }),
            },
            paperFor === p.id ? 'Hide paper kit' : 'Print paper kit',
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
    parts.push(h('div', { class: 'actions' }, flipBtn, fundBtn, kitBtn, paperBtn, calBtn, cancelBtn));
  }
  if (active) {
    if (paperFor === p.id && paperEpoch === p.epoch && paper) parts.push(paperView(p, paper));
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

/**
 * A period picker: common choices, plus "Custom" with a number and a unit.
 * `value()` returns a duration the CLI understands (for example 45d or 12h),
 * or null when the custom number is missing or not a whole number above 0.
 */
function periodPicker(id: string, choices: [string, string][], selected: string, withDate = false): { el: HTMLElement; select: HTMLSelectElement; value: () => string | null } {
  const select = h(
    'select',
    { id },
    ...choices.map(([v, label]) => h('option', { value: v, selected: v === selected ? true : null }, label)),
    withDate ? h('option', { value: 'date' }, 'On a date...') : null,
    h('option', { value: 'custom' }, 'Custom...'),
  );
  const day = h('input', { type: 'date', id: `${id}-d`, 'aria-label': 'Unlock date' });
  const num = h('input', { type: 'number', id: `${id}-n`, min: '1', step: '1', value: '45', 'aria-label': 'Custom amount of time' });
  const unit = h(
    'select',
    { id: `${id}-u`, 'aria-label': 'Unit' },
    h('option', { value: 'm' }, 'minutes'),
    h('option', { value: 'h' }, 'hours'),
    h('option', { value: 'd', selected: true }, 'days'),
    h('option', { value: 'w' }, 'weeks'),
    h('option', { value: 'mo' }, 'months (30 days)'),
  );
  const custom = h('div', { class: 'custom-period' }, num, unit);
  const dayBox = h('div', { class: 'custom-period' }, day);
  const paint = (): void => {
    custom.hidden = select.value !== 'custom';
    dayBox.hidden = select.value !== 'date';
  };
  select.addEventListener('change', paint);
  paint();
  const value = (): string | null => {
    if (select.value === 'date') return dateToUntil(day.value);
    if (select.value !== 'custom') return select.value;
    const n = Number(num.value);
    if (!Number.isInteger(n) || n < 1) return null;
    return unit.value === 'mo' ? `${n * 30}d` : `${n}${unit.value}`;
  };
  return { el: h('div', {}, select, custom, dayBox), select, value };
}

const PERIODS: [string, string][] = [
  ['30d', '30 days'],
  ['90d', '90 days'],
  ['180d', '6 months'],
  ['365d', '1 year'],
];

function kindSwitch(): HTMLElement {
  const btn = (k: 'legacy' | 'installments' | 'lock', title: string, sub: string) =>
    h(
      'button',
      { type: 'button', class: `kind${newKind === k ? ' on' : ''}`, 'aria-pressed': String(newKind === k), onclick: () => ((newKind = k), render()) },
      h('strong', {}, title),
      h('span', {}, sub),
    );
  return h(
    'div',
    { class: 'kind-switch', role: 'group', 'aria-label': 'Plan type' },
    btn('legacy', 'Inheritance', 'Your heirs get the vault if you stop flipping.'),
    btn('installments', 'Scheduled payments', 'An allowance: one installment unlocks per period; cancel any time.'),
    btn('lock', 'Savings lock', 'Lock it from yourself: nobody, you included, can touch it until a date.'),
  );
}

function codeInputs(word: string): HTMLInputElement[] {
  return [0, 1, 2].map((i) =>
    h('input', { type: 'text', id: `code${i}`, placeholder: 'age1...', autocomplete: 'off', spellcheck: 'false', 'aria-label': `${word} code ${i + 1}` }),
  );
}

function newPlanForm(): HTMLElement {
  return newKind === 'legacy' ? legacyForm() : newKind === 'lock' ? lockForm() : installmentsForm();
}

const LOCK_PERIODS: [string, string][] = [
  ['30d', '30 days'],
  ['90d', '90 days'],
  ['180d', '6 months'],
  ['365d', '1 year'],
  ['1825d', '5 years'],
  ['10m', '10 minutes (try it out)'],
];

/**
 * A savings lock: the kit is sealed to the owner's own heir key and the vault
 * seed is deleted at once. The form refuses to send until the owner has ticked
 * the box and typed LOCK.
 */
function lockForm(): HTMLElement {
  const heir = h('input', { type: 'text', id: 'lock-heir', placeholder: 'age1...', autocomplete: 'off', spellcheck: 'false', 'aria-label': 'Your own heir code' });
  const until = periodPicker('lock-until', LOCK_PERIODS, '365d', true);
  const preview = h('p', { class: 'muted small', 'aria-live': 'polite' });
  const paintPreview = (): void => {
    const v = until.value();
    const at = v ? untilEpoch(v, nowSec()) : null;
    preview.textContent = at ? `The lock ends on ${whenText(at)}.` : '';
  };
  until.el.addEventListener('change', paintPreview);
  until.el.addEventListener('input', paintPreview);
  paintPreview();
  const amount = h('input', { type: 'text', id: 'lock-amount', inputmode: 'decimal', placeholder: 'Optional, for example 0.1' });
  const message = h('textarea', { id: 'lock-message', rows: '3', placeholder: 'Optional. A note to your future self, sealed into the kit.' });
  const link = h('input', { type: 'checkbox', id: 'lock-claimlink' });
  const understood = h('input', { type: 'checkbox', id: 'lock-understood' });
  const confirm = h('input', { type: 'text', id: 'lock-confirm', autocomplete: 'off', spellcheck: 'false', placeholder: 'LOCK' });
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card', id: 'new-plan' },
    h('h2', { tabindex: '-1' }, 'New plan'),
    kindSwitch(),
    h(
      'p',
      {},
      'Put shielded ZEC out of your own reach until a date you choose. The kit is sealed to your own heir key: make one on the ',
      h('a', { href: 'https://zandglass.cash/heir', target: '_blank', rel: 'noreferrer noopener' }, 'Become an heir page'),
      ', write down its 24 words, and paste its public heir code below.',
    ),
    notice(
      'warn',
      'A savings lock cannot be undone',
      `The vault seed is deleted from this computer as soon as the kit is sealed. Until the lock ends nobody can move the funds, and that includes you: there is no flip, no cancel and no way to ask for help. Afterwards only two things open the vault: the kit file and your 24 heir words. If you lose either one, the funds are gone for good. Zandglass is unaudited, so keep the amount small (up to ${state ? zec(state.cap_zat) : 'the cap'} on mainnet).`,
    ),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          const code = heir.value.trim();
          if (!code) {
            err.textContent = 'Paste your own heir code (age1...), made on the Become an heir page.';
            return;
          }
          const when = until.value();
          if (!when || !untilEpoch(when, nowSec())) {
            err.textContent = 'Choose how long to lock: a date, or a whole number for the custom length.';
            return;
          }
          if (!understood.checked) {
            err.textContent = 'Tick the box to confirm that you have your heir words.';
            return;
          }
          if (confirm.value.trim() !== 'LOCK') {
            err.textContent = 'Type the word LOCK to confirm.';
            return;
          }
          err.textContent = '';
          void act('Sealing the kit and deleting the seed...', async () => {
            created = await api<Created>('POST', '/api/locks', {
              heirs: [code],
              until: when,
              message: message.value,
              claim_link: link.checked,
              amount: amount.value.trim().replace(',', '.') || null,
              understood: true,
              confirm: 'LOCK',
            });
            showNew = false;
            setState(await api<UiState>('GET', '/api/state'));
            selected = created.plan.id;
          });
        },
      },
      h('label', { for: 'lock-heir' }, 'Your own heir code'),
      heir,
      h('label', { for: 'lock-until' }, 'Locked until'),
      until.el,
      preview,
      h('label', { for: 'lock-amount' }, `Amount to request in the funding QR (${unit()})`),
      amount,
      h('label', { for: 'lock-message' }, 'A note to yourself'),
      message,
      h('label', { class: 'check' }, link, h('span', {}, 'Also keep the kit in a blind mailbox at ', h('strong', {}, 'zandglass.cash'), ' (locks of up to 170 days only: the keeper deletes a mailbox after 180 days). The kit file is what counts.')),
      h('label', { class: 'check' }, understood, h('span', {}, 'I have my 24 heir words written down and I will keep the kit file safe. I understand I cannot get this money back before the lock ends.')),
      h('label', { for: 'lock-confirm' }, 'Type LOCK to confirm'),
      confirm,
      err,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', disabled: busy ? true : null }, 'Create the savings lock')),
    ),
  );
}

function legacyForm(): HTMLElement {
  const heirs = codeInputs('Heir');
  const every = periodPicker('every', [...PERIODS, ['10m', '10 minutes (try it out)']], '90d');
  const message = h('textarea', { id: 'message', rows: '3', placeholder: 'Optional. Sealed into the kit; only your heirs can read it, after the unlock.' });
  const link = h('input', { type: 'checkbox', id: 'claimlink', checked: true });
  const amount = h('input', { type: 'text', id: 'amount', inputmode: 'decimal', placeholder: 'Optional, for example 0.1' });
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card', id: 'new-plan' },
    h('h2', { tabindex: '-1' }, 'New plan'),
    kindSwitch(),
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
          const period = every.value();
          if (!period) {
            err.textContent = 'Enter a whole number for the custom period.';
            return;
          }
          void act('Creating a fresh vault and sealing the kit...', async () => {
            created = await api<Created>('POST', '/api/plans', {
              heirs: codes,
              every: period,
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
      every.el,
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

const INTERVALS: [string, string][] = [
  ['7d', 'Every week'],
  ['14d', 'Every two weeks'],
  ['30d', 'Every 30 days'],
  ['90d', 'Every 90 days'],
  ['5m', 'Every 5 minutes (try it out)'],
];

function installmentsForm(): HTMLElement {
  const payees = codeInputs('Payee');
  const amount = h('input', { type: 'text', id: 'inst-amount', inputmode: 'decimal', placeholder: 'For example 0.05' });
  const every = periodPicker('inst-every', INTERVALS, '30d');
  const count = h('input', { type: 'number', id: 'inst-count', min: '1', max: '120', value: '12' });
  const start = h('input', { type: 'text', id: 'inst-start', placeholder: 'Optional: 2026-12-01T09:00:00Z, or a delay such as 1d' });
  const message = h('textarea', { id: 'inst-message', rows: '3', placeholder: 'Optional. Sealed into every kit for the payee.' });
  const link = h('input', { type: 'checkbox', id: 'inst-claimlink', checked: true });
  const total = h('p', { class: 'muted small', 'aria-live': 'polite' });
  const paintTotal = (): void => {
    const a = Number(amount.value.replace(',', '.'));
    const n = Number(count.value);
    total.textContent = a > 0 && n > 0 ? `The payee receives ${n} × ${a} ${unit()}; you fund about ${(n * (a + 0.0001) + 0.0002).toFixed(4)} ${unit()} including network fees.` : '';
  };
  amount.addEventListener('input', paintTotal);
  count.addEventListener('input', paintTotal);
  const err = h('p', { class: 'field-error', role: 'alert' });
  return h(
    'section',
    { class: 'card', id: 'new-plan' },
    h('h2', { tabindex: '-1' }, 'New plan'),
    kindSwitch(),
    h('p', {}, 'An allowance, a gift over time, or a salary: each installment is its own vault, sealed to the payee and to its date. The payee needs a key from the ', h('a', { href: 'https://zandglass.cash/heir', target: '_blank', rel: 'noreferrer noopener' }, 'Become an heir page'), '.'),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          const codes = payees.map((x) => x.value.trim()).filter(Boolean);
          if (!codes.length) {
            err.textContent = 'Add at least one payee code.';
            return;
          }
          if (!amount.value.trim()) {
            err.textContent = 'Enter the amount per installment.';
            return;
          }
          const interval = every.value();
          if (!interval) {
            err.textContent = 'Enter a whole number for the custom interval.';
            return;
          }
          void act('Creating the funding vault...', async () => {
            created = await api<Created>('POST', '/api/installments', {
              payees: codes,
              amount: amount.value.trim().replace(',', '.'),
              every: interval,
              count: Number(count.value),
              start: start.value.trim() || null,
              message: message.value,
              claim_link: link.checked,
            });
            showNew = false;
            setState(await api<UiState>('GET', '/api/state'));
            selected = created.plan.id;
          });
        },
      },
      h('fieldset', { class: 'heir-fields' }, h('legend', {}, 'Payee codes (one is enough; up to three, any of them can claim)'), ...payees),
      h('div', { class: 'form-row' }, h('div', {}, h('label', { for: 'inst-amount' }, `Each installment (${unit()})`), amount), h('div', {}, h('label', { for: 'inst-count' }, 'Number of installments'), count)),
      h('label', { for: 'inst-every' }, 'How often'),
      every.el,
      h('label', { for: 'inst-start' }, 'First installment'),
      start,
      total,
      h('label', { for: 'inst-message' }, 'A message for the payee'),
      message,
      h('label', { class: 'check' }, link, h('span', {}, 'Put each kit in a blind mailbox at ', h('strong', {}, 'zandglass.cash'), ' and make a file of claim links for the payee.')),
      err,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', disabled: busy ? true : null }, 'Create the schedule')),
    ),
  );
}

function createdCard(c: Created): HTMLElement {
  const inst = c.plan.kind === 'installments';
  const lock = c.plan.kind === 'lock';
  return h(
    'section',
    { class: 'card created-card' },
    h('h2', { tabindex: '-1', id: 'created-title' }, `Plan ${c.plan.short_id} is ready`),
    h('p', {}, inst && c.required_zat ? `1. Fund the plan with ${zec(c.required_zat)}: scan the QR code with any Zcash wallet.` : '1. Fund the vault: scan the QR code with any Zcash wallet.'),
    h(
      'div',
      { class: 'fund-box' },
      h('div', { class: 'qr-tile' }, qrSvg(c.uri, 'QR code of the funding request')),
      h('div', {}, h('code', { class: 'code-box' }, c.address), h('div', { class: 'actions' }, copyButton('Copy address', () => c.address), copyButton('Copy payment request', () => c.uri))),
    ),
    lock
      ? h(
          'div',
          {},
          h('p', {}, `2. Download the kit now and keep copies (the file, a paper print, a password manager). The seed was deleted from this computer: from now on only this kit and your 24 heir words open the vault, after ${c.plan.unlock_time ? whenText(c.plan.unlock_time) : 'the unlock time'}. Lose either and the funds are gone for good.`),
          h('div', { class: 'actions' }, kitButton(c.plan)),
          c.claim_link
            ? h('div', {}, h('p', {}, 'Your claim link (shown only once):'), h('code', { class: 'code-box' }, c.claim_link), h('div', { class: 'actions' }, copyButton('Copy claim link', () => c.claim_link ?? '')))
            : null,
        )
      : inst
      ? h('p', {}, '2. When the funds have confirmed, press "Start the schedule" on the plan. It seals one kit per installment and, if you chose the mailbox, gives you a file of claim links for the payee.')
      : c.claim_link
        ? h(
            'div',
            {},
            h('p', {}, '2. Send this claim link to your heir now. It is shown only once:'),
            h('code', { class: 'code-box' }, c.claim_link),
            h('div', { class: 'actions' }, copyButton('Copy claim link', () => c.claim_link ?? '', 'btn-primary')),
            h('p', { class: 'muted small' }, 'The link opens nothing by itself: your heir also needs their own 24 words, and must wait for the unlock time.'),
          )
        : h('p', {}, '2. Download the kit file below and give it to your heirs.'),
    lock ? h('p', {}, '3. Optional: add the unlock date to your calendar with "Calendar reminder" on the plan.') : inst ? null : h('p', {}, '3. Add the flip reminders to your calendar, and flip before each unlock.'),
    h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn', onclick: () => ((created = null), render()) }, 'Done')),
  );
}

function splitCard(s: SplitDone): HTMLElement {
  return h(
    'section',
    { class: 'card created-card' },
    h('h2', { tabindex: '-1', id: 'split-title' }, `The schedule of plan ${s.plan.short_id} has started`),
    h('p', {}, `One ordinary transaction filled ${s.lines.length} installment vaults. Transaction ${s.txid}`),
    h('ol', { class: 'inst-list' }, ...s.lines.map((l) => h('li', {}, h('span', {}, `Installment ${l.index}`), h('span', { class: 'muted' }, `unlocks ${new Date(l.unlock_time * 1000).toLocaleString()}`)))),
    s.links && s.links_file
      ? h(
          'div',
          {},
          h('p', {}, 'Send the payee this file of claim links. Each link opens nothing before its date, and nothing without the payee’s own words.'),
          h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn-primary', onclick: () => download(s.links_file ?? 'claim-links.txt', s.links ?? '', 'text/plain') }, 'Download the claim links')),
        )
      : h('p', {}, 'Download the kits from the plan and give them to the payee.'),
    h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn', onclick: () => ((splitDone = null), render()) }, 'Done')),
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
      networkSwitch() ?? h('span', { class: state.network === 'test' ? 'badge test' : 'badge' }, state.network === 'test' ? 'Testnet (test coins)' : 'Mainnet (real ZEC)'),
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
  if (splitDone) flow.append(splitCard(splitDone));
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
