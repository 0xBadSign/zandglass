import './style.css';
import './site.css';
import './scenes.css';
import './console.css';
import { clear, copyButton, h, type Child } from './lib/dom';
import { Sandglass, mountDust } from './lib/fx';
import { sceneOpen } from './lib/scenes';
import { BeaconNotReadyError, NetworkError, relayClient } from './lib/drand';
import { generatePhrase, heirCodeFromPhrase } from './lib/heir';
import {
  KitDamagedError,
  WrongHeirKeyError,
  openKit,
  parseKit,
  type OpenedPayload,
} from './lib/kit';
import {
  ApiError,
  MINUTE_CHOICES,
  TERMINAL_STATES,
  apiBase,
  buildTimeline,
  elapsedFraction,
  formatCountdown,
  formatZec,
  isTransient,
  looksLikeUnifiedAddress,
  networkLabel,
  parseClaimLink,
  pollMs,
  request,
  safeHref,
  stateRank,
  type CreatedSession,
  type PlaygroundStatus,
  type SessionSnapshot,
  type StepStatus,
} from './lib/playground';
import { mountShell } from './lib/shell';

const main = mountShell('playground', { wide: true });
mountDust();
const API = apiBase(window.location.search, window.location.origin);
const POLL = pollMs(window.location.search);
const JUDGE_KEY = 'sandglass-judge-code';

type Step = 'code' | 'heir' | 'vault' | 'open';
const STEPS: { id: Step; label: string }[] = [
  { id: 'code', label: 'Judge code' },
  { id: 'heir', label: 'Heir key' },
  { id: 'vault', label: 'Vault' },
  { id: 'open', label: 'Open' },
];

type Problem = { tone: 'bad' | 'warn' | 'info'; title: string; body: string; retry?: () => void };

/* ------------------------------------------------------------ state */

// The heir phrase lives only in this module. It is never stored or sent.
let phrase = '';
let heirCode = '';
let judge = readJudge();
let step: Step = 'code';
let status: PlaygroundStatus | null = null;
let created: CreatedSession | null = null;
let snap: SessionSnapshot | null = null;
let lastRank = 0;
let stopFlip = false;
let busy = false;
let starting = false;
let opening = false;
let opened: OpenedPayload | null = null;
let seedShown = false;
let codeError = '';
let pollTimer: number | undefined;

// Persistent live elements, re-attached by render() so polling never steals focus.
const problemBox = h('div', { id: 'problems' });
const countdownEl = h('p', { class: 'countdown big', role: 'timer', id: 'countdown' });
const glass = new Sandglass({ className: 'pg-glass' });
glass.set(0);
glass.setFlowing(false);
const stageNet = h('span', { class: 'badge', id: 'stage-net' });
const stageCaption = h('p', { class: 'stage-caption' });
const vaultRow = h('ol', { class: 'vault-row', 'aria-label': 'Vaults' });
const stage = h(
  'aside',
  { class: 'pg-stage', 'aria-label': 'Vault status' },
  h('div', { class: 'stage-top' }, h('span', { class: 'stage-k' }, 'Live vault'), stageNet),
  h('div', { class: 'stage-glass' }, h('div', { class: 'halo', 'aria-hidden': 'true' }), glass.el),
  countdownEl,
  stageCaption,
  h('div', { class: 'stage-vaults' }, h('span', { class: 'stage-k' }, 'Vault epochs'), vaultRow),
);
const timelineBox = h('div', { id: 'timeline-box' });
const messageEl = h('p', { class: 'muted', role: 'status', id: 'session-message' });
const flipHint = h('p', { class: 'muted small', id: 'flip-hint' });
const flipBtn = h('button', { type: 'button', class: 'btn-primary', id: 'flip' }, "Flip (I'm still here)");
const stopBtn = h('button', { type: 'button', class: 'btn', id: 'stop' }, 'Stop flipping');

function readJudge(): string {
  try {
    return sessionStorage.getItem(JUDGE_KEY) ?? '';
  } catch {
    return '';
  }
}

function saveJudge(v: string): void {
  try {
    sessionStorage.setItem(JUDGE_KEY, v);
  } catch {
    // storage may be blocked; the code then lives in memory only
  }
}

/* ------------------------------------------------------------ helpers */

function networkName(): string {
  return created?.network ?? status?.network ?? 'main';
}

function setProblem(p: Problem | null): void {
  clear(problemBox);
  if (!p) return;
  problemBox.append(
    h(
      'div',
      { class: `notice ${p.tone}`, role: p.tone === 'bad' ? 'alert' : 'status' },
      h(
        'div',
        {},
        h('strong', {}, p.title),
        h('p', {}, p.body),
        p.retry ? h('button', { type: 'button', class: 'btn', onclick: p.retry }, 'Try again') : null,
      ),
    ),
  );
}

function apiProblem(e: unknown, title: string): Problem {
  const msg = e instanceof ApiError ? e.message : 'Something went wrong. Try again in a moment.';
  const net = e instanceof ApiError && e.code === 'network';
  return { tone: net ? 'warn' : 'bad', title, body: msg };
}

function go(next: Step): void {
  step = next;
  render();
  main.querySelector<HTMLElement>('h2')?.focus();
  window.scrollTo({ top: 0 });
}

function resetAll(): void {
  window.clearTimeout(pollTimer);
  phrase = '';
  heirCode = '';
  created = null;
  snap = null;
  lastRank = 0;
  stopFlip = false;
  busy = false;
  starting = false;
  opening = false;
  opened = null;
  seedShown = false;
  setProblem(null);
  go('heir');
}

/* ------------------------------------------------------------ chrome */

function banner(): HTMLElement {
  const net = networkLabel(networkName());
  const testnet = net.test;
  return h(
    'div',
    { class: 'notice info demo-banner', role: 'note' },
    h(
      'div',
      {},
      h(
        'strong',
        {},
        testnet ? 'Testnet demo' : 'Mainnet demo',
        ' ',
        h('span', { class: net.test ? 'badge test' : 'badge', id: 'net-badge' }, net.text),
      ),
      h(
        'p',
        {},
        'Here the server plays the owner and holds the demo vault. In real use the owner runs Sandglass on their own device. ' +
          'Your heir key never leaves this browser.',
      ),
    ),
  );
}

function stepper(): HTMLElement {
  const idx = STEPS.findIndex((s) => s.id === step);
  return h(
    'ol',
    { class: 'steps', 'aria-label': 'Progress' },
    ...STEPS.map((s, i) =>
      h('li', { class: i < idx ? 'done' : '', 'aria-current': i === idx ? 'step' : null }, `${i + 1}. ${s.label}`),
    ),
  );
}

function statusLine(): HTMLElement | null {
  if (!status) return null;
  if (!status.enabled) {
    return h(
      'div',
      { class: 'notice warn', role: 'status' },
      h('div', {}, h('strong', {}, 'The playground is closed right now'), h('p', {}, 'Please come back later.')),
    );
  }
  return h(
    'p',
    { class: 'muted small', id: 'status-line' },
    `Each session uses ${formatZec(status.amount_zat, networkName() === 'test' ? 'TAZ' : 'ZEC')} from the demo wallet. ` +
      `${status.active} of ${status.max_active} session slots are busy.`,
  );
}

/* ------------------------------------------------------------ step 1: judge code */

function codeView(): HTMLElement {
  const input = h('input', {
    type: 'password',
    id: 'judge-code',
    name: 'judge-code',
    autocomplete: 'off',
    autocapitalize: 'none',
    autocorrect: 'off',
    spellcheck: 'false',
    'aria-describedby': 'judge-help',
  });
  input.value = judge;
  const error = h('p', { class: 'field-error', role: 'alert' }, codeError || null);
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, '1. Enter the judge code'),
    h(
      'p',
      { id: 'judge-help' },
      'The playground uses a small demo wallet, so it needs the judge code from the submission. ' +
        'The code is kept only for this browser tab and is sent only to the playground server.',
    ),
    statusLine(),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          const v = input.value.trim();
          if (!v) {
            error.textContent = 'Enter the judge code to continue.';
            return;
          }
          judge = v;
          codeError = '';
          saveJudge(v);
          go('heir');
        },
      },
      h('label', { for: 'judge-code' }, 'Judge code'),
      input,
      error,
      h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary' }, 'Continue')),
    ),
  );
}

/* ------------------------------------------------------------ step 2: heir */

function heirView(): HTMLElement {
  const parts: Child[] = [
    h('h2', { tabindex: '-1' }, '2. Become the heir'),
    h(
      'p',
      {},
      'You play the person who will inherit. Your heir key is made here, in your browser, from 24 words. ' +
        'The server only ever receives the public heir code.',
    ),
  ];
  if (!heirCode) {
    parts.push(
      h(
        'div',
        { class: 'actions' },
        h(
          'button',
          {
            type: 'button',
            class: 'btn-primary',
            onclick: () => {
              phrase = generatePhrase();
              void heirCodeFromPhrase(phrase).then((c) => {
                heirCode = c;
                render();
                main.querySelector<HTMLElement>('h2')?.focus();
              });
            },
          },
          'Create my heir key',
        ),
      ),
    );
  } else {
    parts.push(
      h('h3', {}, 'Your 24 words'),
      h('ol', { class: 'phrase', 'aria-label': 'Your 24 words' }, ...phrase.split(' ').map((w) => h('li', {}, w))),
      h(
        'div',
        { class: 'notice warn', role: 'note' },
        h(
          'div',
          {},
          h('strong', {}, 'Demo shortcut'),
          h(
            'p',
            {},
            'For this demo you can skip writing the words down and the 3-word check. This page keeps them in memory so it can open your kit later. ' +
              'Reloading the page forgets them. For a real inheritance, write them on paper and confirm them on the ',
            h('a', { href: './heir' }, 'Become an heir page'),
            '.',
          ),
        ),
      ),
      h(
        'div',
        { class: 'key-card' },
        h('span', { class: 'kc-chip', 'aria-hidden': 'true' }),
        h('h3', { class: 'kc-label' }, 'Your heir code (public)'),
        h('code', { class: 'code-box', id: 'heir-code' }, heirCode),
        h('span', { class: 'kc-fine' }, 'Safe to share. Only your 24 words can use it.'),
      ),
      h(
        'div',
        { class: 'actions' },
        copyButton('Copy heir code', () => heirCode),
        h('button', { type: 'button', class: 'btn-primary', id: 'use-heir', onclick: () => go('vault') }, 'Use this heir code'),
      ),
    );
  }
  return h('section', { class: 'card' }, ...parts);
}

/* ------------------------------------------------------------ step 3: vault */

function choice(name: string, value: string, title: string, desc: string, checked: boolean): HTMLElement {
  return h(
    'label',
    { class: 'choice' },
    h('input', { type: 'radio', name, value, checked: checked ? true : null }),
    h('span', {}, h('strong', {}, title), h('span', { class: 'muted small' }, desc)),
  );
}

async function startSession(minutes: number): Promise<void> {
  if (starting) return;
  starting = true;
  setProblem(null);
  render();
  try {
    created = await request<CreatedSession>(API, '/v1/playground/sessions', {
      method: 'POST',
      body: { judge_code: judge, heir_code: heirCode, minutes },
    });
    snap = null;
    lastRank = 0;
    stopFlip = false;
    starting = false;
    render();
    void poll();
  } catch (e) {
    starting = false;
    if (e instanceof ApiError && e.code === 'judge_code_invalid') {
      codeError = e.message;
      go('code');
      return;
    }
    setProblem(apiProblem(e, 'Could not start the vault'));
    render();
  }
}

function startView(): HTMLElement {
  const unit = networkName() === 'test' ? 'TAZ' : 'ZEC';
  const amount = status ? formatZec(status.amount_zat, unit) : `a tiny amount of ${unit}`;
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, '3. Start a vault'),
    h(
      'p',
      {},
      `The server (playing the owner) funds a new vault with ${amount} from the demo wallet, locks it to your heir code, ` +
        'and stores the kit in a mailbox. Pick how long the timer runs.',
    ),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          const v = (e.target as HTMLFormElement).querySelector<HTMLInputElement>('input[name="minutes"]:checked');
          void startSession(Number(v?.value ?? 5));
        },
      },
      h(
        'fieldset',
        { class: 'choices-box' },
        h('legend', {}, 'Timer'),
        ...MINUTE_CHOICES.map((m) => choice('minutes', String(m), `${m} minutes`, 'Until your heir can open the vault.', m === 5)),
      ),
      h(
        'div',
        { class: 'actions' },
        h(
          'button',
          { type: 'submit', class: 'btn-primary', id: 'start', disabled: starting ? true : null },
          starting ? 'Starting...' : 'Start the vault',
        ),
      ),
      starting
        ? h('p', { class: 'muted', role: 'status' }, h('span', { class: 'spinner' }), ' Creating the vault and sealing the kit.')
        : null,
    ),
  );
}

/* ------------------------------------------------------------ live session */

const STATUS_TEXT: Record<StepStatus, string> = {
  done: 'Done',
  active: 'Now',
  todo: 'Waiting',
  skipped: 'Skipped',
  failed: 'Failed',
};

function shortTxid(txid: string): string {
  return txid.length > 16 ? `${txid.slice(0, 8)}...${txid.slice(-6)}` : txid;
}

function txLink(t: { kind: string; txid: string; explorer: string }, full = false): HTMLElement {
  const href = safeHref(t.explorer);
  const text = full ? t.txid : `${t.kind} ${shortTxid(t.txid)}`;
  const link = href
    ? h('a', { href, rel: 'noreferrer noopener', target: '_blank', class: 'txlink', title: t.txid }, text, h('span', { class: 'sr-only' }, ' (opens in a new tab)'))
    : h('code', { title: t.txid }, text);
  if (full) return link;
  return h('span', { class: 'tx-item' }, link, copyButton('Copy', () => t.txid, 'btn-quiet tx-copy'));
}

function paintTimeline(): void {
  const items = buildTimeline(snap, lastRank);
  timelineBox.replaceChildren(
    h(
      'ol',
      { class: 'timeline', 'aria-label': 'Vault timeline' },
      ...items.map((s) =>
        h(
          'li',
          { class: `tl ${s.status}`, 'data-step': s.id },
          h('span', { class: 'tl-head' }, h('span', { class: 'tl-label' }, s.label), h('span', { class: 'tl-status' }, STATUS_TEXT[s.status])),
          h('span', { class: 'tl-detail muted small' }, s.detail),
          s.txs.length ? h('span', { class: 'tl-txs small' }, ...s.txs.map((t) => txLink(t))) : null,
        ),
      ),
    ),
  );
  messageEl.textContent = snap?.message ?? 'Starting your vault...';
}

function paintButtons(): void {
  const canFlip = snap?.state === 'sealed' && snap.confirmed && !stopFlip && !busy;
  flipBtn.disabled = !canFlip;
  stopBtn.textContent = stopFlip ? 'Resume flipping' : 'Stop flipping';
  stopBtn.disabled = busy || snap?.state === 'funding' || (snap !== null && stateRank(snap.state) >= 3);
  let hint = '';
  if (!snap || snap.state === 'funding') hint = 'You can flip once the funding has confirmed on the network.';
  else if (snap.state === 'flipping') hint = 'Flipping: moving the funds to a new vault and sealing a new kit.';
  else if (snap.state === 'sealed' && !snap.confirmed)
    hint =
      snap.txs.some((t) => t.kind === 'flip')
        ? 'Flipped. Waiting for the moved funds to confirm on the network (about a minute); you can flip again after that.'
        : 'Waiting for the funding to confirm on the network (about a minute).';
  else if (stopFlip) hint = 'You stopped flipping, so the sand will run out at the time shown above.';
  else if (snap.state === 'sealed') hint = 'Flip any time before the sand runs out to reset the timer. Or stop flipping and watch it run out.';
  flipHint.textContent = hint;
}

const IDLE_CAPTION: Record<Step, string> = {
  code: 'Enter the judge code to begin.',
  heir: 'Make the heir key that will open this vault.',
  vault: 'Start a vault to set the sand running.',
  open: 'The sand has run out.',
};

function cd(label: string, time: string): void {
  countdownEl.replaceChildren(h('span', { class: 'cd-label' }, label), h('span', { class: 'cd-time' }, time));
}

/** The left-hand stage: the sandglass, the countdown and the chain of vaults. */
function paintStage(): void {
  const net = networkLabel(networkName());
  stageNet.textContent = net.text;
  stageNet.className = net.test ? 'badge test' : 'badge';

  const flips = snap ? snap.txs.filter((t) => t.kind === 'flip').length : 0;
  const count = created ? 1 + flips : 0;
  const unlocked = snap !== null && stateRank(snap.state) >= 3 && snap.state !== 'failed';
  vaultRow.replaceChildren(
    ...(count
      ? Array.from({ length: count }, (_, i) =>
          h(
            'li',
            { class: i < count - 1 ? 'old' : unlocked ? 'cur open' : 'cur', title: i < count - 1 ? 'Emptied by a flip' : 'Holds the funds' },
            h('span', { class: 'v-n' }, `#${i + 1}`),
            h('span', { class: 'v-s' }, i < count - 1 ? 'empty' : unlocked ? 'opened' : 'funds'),
          ),
        )
      : [h('li', { class: 'none' }, 'No vault yet')]),
  );

  if (!created || !snap || snap.state === 'funding') {
    glass.set(0);
    glass.setFlowing(false);
    stageCaption.textContent = created ? 'Funding a fresh vault on chain.' : IDLE_CAPTION[step];
    if (!created) cd('Timer', '--m --s');
    return;
  }
  if (unlocked || TERMINAL_STATES.includes(snap.state)) {
    glass.set(1);
    glass.setFlowing(false);
    stageCaption.textContent = opened ? 'The kit is open in this browser.' : 'The sand has run out. The heir can open the kit.';
    cd('Unlocked', '00m 00s');
    return;
  }
  if (snap.unlock_time) {
    const frac = elapsedFraction(Date.parse(snap.unlock_time), created.minutes, Date.now());
    glass.set(frac);
    glass.setFlowing(snap.state === 'sealed' && frac < 1);
  }
  stageCaption.textContent =
    snap.state === 'flipping'
      ? 'Flipping: the funds move to a new vault.'
      : stopFlip
        ? 'You stopped flipping. Let the sand run out.'
        : 'The sand is falling. A flip turns the glass.';
}

function paintCountdown(): void {
  if (!snap || (snap.state !== 'sealed' && snap.state !== 'flipping' && snap.state !== 'funding')) return;
  if (snap.state === 'funding' || !snap.unlock_time) {
    countdownEl.textContent = 'Funding the vault...';
    return;
  }
  const left = (Date.parse(snap.unlock_time) - Date.now()) / 1000;
  if (left > 0) {
    cd('Unlocks in ', formatCountdown(left));
  } else {
    countdownEl.textContent = 'The time has passed. Waiting for the server to confirm...';
  }
}

function applySnapshot(s: SessionSnapshot): void {
  const before = snap?.state;
  snap = s;
  if (s.state !== 'failed') lastRank = Math.max(lastRank, stateRank(s.state));
  paintTimeline();
  paintButtons();
  paintStage();
  paintCountdown();
  const unlocked = stateRank(s.state) >= 3 && s.state !== 'failed';
  if (step === 'vault' && created && unlocked) go('open');
  else if (step === 'open') {
    if (before !== s.state) render();
  } else if (step === 'vault' && s.state === 'failed') {
    render();
  }
}

async function poll(): Promise<void> {
  window.clearTimeout(pollTimer);
  if (!created) return;
  try {
    const s = await request<SessionSnapshot>(API, `/v1/playground/sessions/${encodeURIComponent(created.id)}`, {
      token: created.token,
    });
    applySnapshot(s);
    if (problemBox.textContent?.includes('Connection problem')) setProblem(null);
  } catch (e) {
    if (!isTransient(e)) {
      setProblem(apiProblem(e, 'This session cannot continue'));
      return;
    }
    setProblem({
      tone: 'warn',
      title: 'Connection problem',
      body: 'Could not reach the playground server. Retrying by itself. Your vault is not affected.',
    });
  }
  if (!snap || !TERMINAL_STATES.includes(snap.state)) {
    pollTimer = window.setTimeout(() => void poll(), POLL);
  }
}

async function act<T>(fn: () => Promise<T>, title: string): Promise<T | null> {
  if (busy) return null;
  busy = true;
  setProblem(null);
  paintButtons();
  try {
    return await fn();
  } catch (e) {
    setProblem(apiProblem(e, title));
    return null;
  } finally {
    busy = false;
    paintButtons();
  }
}

flipBtn.addEventListener('click', () => {
  if (!created) return;
  const c = created;
  void act(async () => {
    const before = snap;
    if (snap) applySnapshot({ ...snap, state: 'flipping', message: 'Flipping: moving the funds to a new vault.' });
    try {
      const s = await request<SessionSnapshot>(API, `/v1/playground/sessions/${encodeURIComponent(c.id)}/flip`, {
        method: 'POST',
        token: c.token,
        body: {},
      });
      void glass.flip();
      applySnapshot(s);
    } catch (e) {
      if (before) applySnapshot(before);
      throw e;
    }
  }, 'Could not flip');
});

stopBtn.addEventListener('click', () => {
  stopFlip = !stopFlip;
  paintButtons();
  paintStage();
});

setInterval(() => {
  paintCountdown();
  paintStage();
}, 1000);

function sessionView(): HTMLElement {
  paintTimeline();
  paintButtons();
  paintStage();
  paintCountdown();
  const net = networkLabel(networkName());
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, '3. Your vault is running'),
    h(
      'p',
      { class: 'muted' },
      h('span', { class: net.test ? 'badge test' : 'badge' }, net.text),
      ` Timer: ${created?.minutes ?? ''} minutes. Amount: ${formatZec(created?.amount_zat ?? 0, net.test ? 'TAZ' : 'ZEC')}.`,
    ),
    messageEl,
    h(
      'p',
      { class: 'keep-open small' },
      'Keep this page open: reloading forgets your heir key, and you could no longer open the kit.',
    ),
    h('div', { class: 'actions' }, flipBtn, stopBtn),
    flipHint,
    h('h3', {}, 'What is happening'),
    timelineBox,
  );
}

/* ------------------------------------------------------------ step 4: open */

async function fetchKitText(): Promise<string> {
  const link = parseClaimLink(snap?.claim_link ?? created?.claim_link ?? '');
  if (!link) throw new ApiError('mailbox_not_found', 404, 'The claim link is missing.');
  let res: Response;
  try {
    res = await fetch(`${API}/v1/mailboxes/${encodeURIComponent(link.id)}/kit`, {
      headers: { Authorization: `Bearer ${link.token}` },
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
    });
  } catch {
    throw new NetworkError('keeper unreachable');
  }
  if (res.status === 404 || res.status === 401 || res.status === 403) {
    throw new ApiError('mailbox_not_found', res.status, 'The kit mailbox was not found.');
  }
  if (!res.ok) throw new NetworkError(`keeper answered ${res.status}`);
  const text = await res.text();
  if (text.length > 1024 * 1024) throw new KitDamagedError();
  return text;
}

async function openHere(): Promise<void> {
  if (opening || opened) return;
  opening = true;
  setProblem(null);
  render();
  try {
    const kit = parseKit(await fetchKitText());
    opened = await openKit(kit, phrase, relayClient());
    seedShown = false;
  } catch (e) {
    opened = null;
    if (e instanceof ApiError) {
      setProblem({ tone: 'bad', title: 'Kit not found', body: 'The kit mailbox could not be found. Sessions end by themselves after a while. Start a new one.' });
    } else if (e instanceof BeaconNotReadyError) {
      setProblem({
        tone: 'info',
        title: 'The time key is not published yet',
        body: 'The unlock time has just passed and the drand network is about to publish the key. Try again in a few seconds.',
        retry: () => void openHere(),
      });
    } else if (e instanceof NetworkError) {
      setProblem({
        tone: 'warn',
        title: 'Could not reach the server or the drand time servers',
        body: 'Check your internet connection and try again. Your heir words were not sent anywhere.',
        retry: () => void openHere(),
      });
    } else if (e instanceof WrongHeirKeyError) {
      setProblem({
        tone: 'bad',
        title: 'These words do not open this kit',
        body: 'The kit was sealed to a different heir code. Reloading the page creates a new heir key, so start a new session.',
      });
    } else {
      setProblem({
        tone: 'bad',
        title: 'This kit looks damaged',
        body: 'The time lock opened, but what is inside could not be read. Start a new session.',
      });
    }
  }
  opening = false;
  render();
  if (opened) document.getElementById('opened-title')?.focus();
}

function openedView(r: OpenedPayload): HTMLElement {
  const payload = r.kind === 'json' ? r.payload : null;
  const seed = payload ? payload.vault_seed : r.kind === 'raw' ? r.text : '';
  const seedBox = h('div', { class: 'seed hidden', id: 'seed', 'aria-live': 'polite' });
  const toggle = h('button', { type: 'button', class: 'btn', 'aria-pressed': 'false' }, 'Show the seed');
  const paint = (): void => {
    seedBox.className = seedShown ? 'seed' : 'seed hidden';
    seedBox.textContent = seedShown ? seed : seed.split(/\s+/).map(() => '••••').join(' ');
    toggle.textContent = seedShown ? 'Hide the seed' : 'Show the seed';
    toggle.setAttribute('aria-pressed', String(seedShown));
  };
  toggle.addEventListener('click', () => {
    seedShown = !seedShown;
    paint();
  });
  paint();
  return h(
    'div',
    { class: 'opened' },
    h('div', { class: 'opened-art', 'aria-hidden': 'true' }, sceneOpen()),
    h('h3', { tabindex: '-1', id: 'opened-title' }, 'The kit is open'),
    h(
      'div',
      { class: 'notice ok', role: 'status' },
      h(
        'div',
        {},
        h('strong', {}, 'Opened here, in your browser'),
        h('p', {}, 'Both locks were opened on this device with your heir key. Nothing you typed or saw here was sent anywhere.'),
      ),
    ),
    payload?.message ? h('blockquote', { class: 'message' }, payload.message) : null,
    h('h3', {}, 'Vault seed (24 words)'),
    seedBox,
    h('div', { class: 'actions' }, toggle),
    payload ? h('h3', {}, 'Birthday height') : null,
    payload ? h('p', { class: 'kv', id: 'birthday' }, payload.birthday_height.toLocaleString('en-US')) : null,
  );
}

function settleView(): HTMLElement {
  const net = networkName();
  const input = h('input', {
    type: 'text',
    id: 'send-to',
    name: 'send-to',
    autocomplete: 'off',
    autocapitalize: 'none',
    autocorrect: 'off',
    spellcheck: 'false',
    placeholder: net === 'test' ? 'utest1...' : 'u1...',
    'aria-describedby': 'send-help',
  });
  const error = h('p', { class: 'field-error', role: 'alert' });
  const form = h(
    'form',
    {
      novalidate: true,
      onsubmit: (e: Event) => {
        e.preventDefault();
        const action = form.querySelector<HTMLInputElement>('input[name="settle"]:checked')?.value ?? 'return';
        error.textContent = '';
        if (action === 'send' && !looksLikeUnifiedAddress(input.value, net)) {
          error.textContent = `Enter a unified address for ${net === 'test' ? 'testnet (starts with utest1)' : 'mainnet (starts with u1)'}.`;
          input.focus();
          return;
        }
        void settle(action, input.value.trim());
      },
    },
    h(
      'fieldset',
      { class: 'choices-box' },
      h('legend', {}, 'What should happen to the coins?'),
      choice('settle', 'return', 'Return the coins to the demo', 'The server sweeps the vault back to the demo wallet. Recommended.', true),
      choice('settle', 'send', 'Send to my address', 'The server sweeps the vault to a unified address you enter.', false),
      h('div', { class: 'indent' }, h('label', { for: 'send-to' }, 'Your unified address'), input, h('p', { class: 'muted small', id: 'send-help' }, 'Used only if you choose "Send to my address".'), error),
      choice('settle', 'keep', 'Keep it (restore in your own wallet)', 'You restore the seed above in your own wallet. The number of kept vaults per day is limited.', false),
    ),
    h('div', { class: 'actions' }, h('button', { type: 'submit', class: 'btn-primary', id: 'settle-go', disabled: busy ? true : null }, 'Confirm')),
  );
  return h(
    'div',
    {},
    h('h3', {}, 'Choose what happens next'),
    form,
    h('p', { class: 'muted small' }, 'If you do nothing, the server returns the coins to the demo wallet 30 minutes after the unlock.'),
  );
}

async function settle(action: string, to: string): Promise<void> {
  if (!created) return;
  const c = created;
  const body = action === 'send' ? { action, to } : { action };
  const s = await act(
    () =>
      request<SessionSnapshot>(API, `/v1/playground/sessions/${encodeURIComponent(c.id)}/settle`, {
        method: 'POST',
        token: c.token,
        body,
      }),
    'Could not finish',
  );
  if (s) {
    applySnapshot(s);
    render();
    if (!TERMINAL_STATES.includes(s.state)) void poll();
  }
}

function outcomeView(s: SessionSnapshot): HTMLElement {
  const sym = networkName() === 'test' ? 'TAZ' : 'ZEC';
  const text: Record<string, string> = {
    settling: 'Settling: the server is moving the coins. This takes a moment.',
    returned: 'Done. The coins went back to the demo wallet.',
    sent: 'Done. The coins were sent to your address.',
    kept: `Done. The vault is yours. Restore the seed in a Zcash wallet (Zodl, Zkool, Vizor or Zingo), enter the birthday height, and sync to see ${formatZec(s.balance_zat, sym)}.`,
    failed: s.error ?? 'The session failed. The demo wallet will recover the coins by itself.',
  };
  const tone = s.state === 'failed' ? 'bad' : s.state === 'settling' ? 'info' : 'ok';
  return h(
    'div',
    {},
    h('h3', {}, 'Final state'),
    h(
      'div',
      { class: `notice ${tone}`, role: 'status', id: 'outcome' },
      h('div', {}, h('strong', {}, `State: ${s.state}`), h('p', {}, text[s.state] ?? s.message)),
    ),
    s.state === 'settling' ? h('p', { class: 'muted' }, h('span', { class: 'spinner' }), ' Waiting for the network...') : null,
    s.txs.length
      ? h(
          'div',
          {},
          h('h3', {}, 'Transactions'),
          h(
            'ul',
            { class: 'txlist' },
            ...s.txs.map((t) => h('li', {}, h('span', { class: 'tx-kind' }, t.kind), txLink(t, true))),
          ),
        )
      : null,
    TERMINAL_STATES.includes(s.state)
      ? h('div', { class: 'actions' }, h('button', { type: 'button', class: 'btn-primary', id: 'again', onclick: resetAll }, 'Try again with a new heir key'))
      : null,
  );
}

function openView(): HTMLElement {
  const s = snap;
  const settled = s !== null && ['settling', 'returned', 'sent', 'kept', 'failed'].includes(s.state);
  const parts: Child[] = [h('h2', { tabindex: '-1' }, '4. Open the kit')];
  if (!opened && !settled) {
    parts.push(
      h('div', { class: 'notice ok', role: 'status' }, h('div', {}, h('strong', {}, 'The sand has run out'), h('p', {}, 'The time lock has passed. Open the kit with the heir key that is held in this tab.'))),
      h(
        'p',
        {},
        'This fetches the kit from the mailbox and the time key from drand, and opens both locks here in your browser.',
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { type: 'button', class: 'btn-primary', id: 'open-kit', disabled: opening ? true : null, onclick: () => void openHere() }, opening ? 'Opening...' : 'Open the kit'),
      ),
      opening ? h('p', { class: 'muted', role: 'status' }, h('span', { class: 'spinner' }), ' Fetching the time key and opening both layers.') : null,
    );
  }
  if (opened) parts.push(openedView(opened));
  if (s && settled) parts.push(outcomeView(s));
  else if (opened) parts.push(settleView());
  parts.push(h('h3', {}, 'What happened'), timelineBox);
  paintTimeline();
  return h('section', { class: 'card' }, ...parts);
}

/* ------------------------------------------------------------ render */

function render(): void {
  clear(main);
  const flow = h(
    'div',
    { class: 'pg-flow' },
    h('span', { class: 'eyebrow' }, 'A real vault, in about five minutes'),
    h('h1', {}, 'Play', h('em', {}, 'ground')),
    h('p', { class: 'lead' }, 'Try the whole flow in about five minutes. You are the heir; the server plays the owner.'),
    banner(),
    stepper(),
    problemBox,
  );
  if (step === 'code') flow.append(codeView());
  else if (step === 'heir') flow.append(heirView());
  else if (step === 'vault') flow.append(created ? sessionView() : startView());
  else flow.append(openView());
  main.append(h('div', { class: `pg-console step-${step}` }, stage, flow));
  paintStage();
}

/* ------------------------------------------------------------ start */

render();
void request<PlaygroundStatus>(API, '/v1/playground/status')
  .then((s) => {
    status = s;
    if (step === 'code' || step === 'heir' || (step === 'vault' && !created)) {
      const focused = document.activeElement?.id;
      const typed = (document.getElementById('judge-code') as HTMLInputElement | null)?.value;
      render();
      const el = focused ? document.getElementById(focused) : null;
      if (el instanceof HTMLInputElement && typed !== undefined) el.value = typed;
      el?.focus();
    }
  })
  .catch((e: unknown) => {
    setProblem({
      tone: 'warn',
      title: 'Cannot reach the playground server',
      body: e instanceof ApiError ? e.message : 'Try again later.',
    });
  });
