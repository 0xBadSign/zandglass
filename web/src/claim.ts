import './style.css';
import './site.css';
import './scenes.css';
import './console.css';
import { clear, copyButton, h, type Child } from './lib/dom';
import { GlassCanvas, mountDust } from './lib/fx';
import { sceneKit, sceneOpen } from './lib/scenes';
import { BeaconNotReadyError, NetworkError, relayClient } from './lib/drand';
import { InvalidPhraseError, isValidPhrase } from './lib/heir';
import {
  KitDamagedError,
  WrongHeirKeyError,
  openKit,
  parseKit,
  unlockSeconds,
  type LoadedKit,
  type OpenedPayload,
} from './lib/kit';
import { acceptLocalKeeper } from './lib/local-url';
import { mountShell } from './lib/shell';

const MAX_KIT_BYTES = 1024 * 1024;

const main = mountShell('claim', { wide: true });
mountDust();

type Source = { kind: 'mailbox'; origin: string; id: string; token: string } | { kind: 'file'; name: string };

type Problem = { tone: 'bad' | 'warn' | 'info'; title: string; body: string; retry?: () => void };

let kit: LoadedKit | null = null;
let source: Source | null = null;
let loading = false;
let opening = false;
let problem: Problem | null = null;
let result: OpenedPayload | null = null;
let seedShown = false;
let timer: number | undefined;

// The phrase field is created once and never persisted or logged.
const phraseEl = h('textarea', {
  id: 'phrase',
  name: 'heir-phrase',
  rows: '4',
  autocomplete: 'off',
  autocapitalize: 'none',
  autocorrect: 'off',
  spellcheck: 'false',
  'aria-describedby': 'phrase-help',
});

const countdownEl = h('p', { class: 'countdown', id: 'countdown' });
const lateTimeEl = h('span', { class: 'sr-only', 'aria-live': 'polite' });

/* ------------------------------------------------------------ stage */

const stageGlass = new GlassCanvas({ className: 'pg-glass' });
const stageNet = h('span', { class: 'badge' }, 'No kit');
const stageCaption = h('p', { class: 'stage-caption' });
const stageBody = h('div', { class: 'stage-body' });
const stageTime = h('p', { class: 'countdown', 'aria-hidden': 'true' });
const stage = h(
  'aside',
  { class: 'app-stage claim-stage', 'aria-label': 'Your kit' },
  h('div', { class: 'stage-top' }, h('span', { class: 'stage-k' }, 'Kit'), stageNet),
  stageBody,
  stageTime,
  stageCaption,
);
let stageMode = '';

function paintStage(): void {
  const mode = !kit ? 'none' : result ? 'open' : isUnlocked(kit) ? 'ready' : 'locked';
  if (kit) {
    const net = networkLabel(kit);
    stageNet.textContent = net.text;
    stageNet.className = net.test ? 'badge test' : 'badge';
  }
  if (mode !== stageMode) {
    stageMode = mode;
    stage.dataset.mode = mode;
    if (mode === 'locked') stageBody.replaceChildren(h('div', { class: 'stage-glass' }, h('div', { class: 'halo' }), stageGlass.el));
    else if (mode === 'open') stageBody.replaceChildren(sceneOpen());
    else {
      const s = sceneKit();
      if (mode === 'ready') s.classList.add('ready');
      stageBody.replaceChildren(s);
    }
  }
  if (mode === 'locked' && kit) {
    const left = unlockSeconds(kit) - nowSeconds();
    stageGlass.set(Math.min(0.97, Math.max(0.03, 1 - left / (86400 * 30))));
    stageGlass.setFlowing(true);
    stageTime.replaceChildren(h('span', { class: 'cd-label' }, 'Unlocks in'), h('span', { class: 'cd-time' }, formatLeft(left)));
  } else {
    stageTime.replaceChildren();
  }
  stageCaption.textContent = {
    none: 'Open the claim link the owner sent you, or a kit file.',
    locked: `Sealed to drand round ${kit?.round.toLocaleString('en-US') ?? ''}. Nobody can open it before then.`,
    ready: 'The round has arrived. Your heir phrase opens the seal.',
    open: 'Opened in this browser. Nothing you typed was sent anywhere.',
  }[mode];
}

/* ------------------------------------------------------------ helpers */

function nowSeconds(): number {
  return Date.now() / 1000;
}

function formatLeft(total: number): string {
  const s = Math.max(0, Math.ceil(total));
  const d = Math.floor(s / 86400);
  const hh = Math.floor((s % 86400) / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const p = (n: number) => String(n).padStart(2, '0');
  if (d > 0) return `${d}d ${p(hh)}h ${p(mm)}m ${p(ss)}s`;
  if (hh > 0) return `${hh}h ${p(mm)}m ${p(ss)}s`;
  return `${mm}m ${p(ss)}s`;
}

function formatUnlock(sec: number): { local: string; utc: string } {
  const d = new Date(sec * 1000);
  return {
    local: d.toLocaleString(undefined, { dateStyle: 'full', timeStyle: 'short' }),
    utc: d.toISOString().replace('.000Z', 'Z').replace('T', ' ').replace('Z', ' UTC'),
  };
}

function isUnlocked(k: LoadedKit): boolean {
  return nowSeconds() >= unlockSeconds(k);
}

function networkLabel(k: LoadedKit): { text: string; test: boolean } {
  const n = k.meta?.network;
  if (n === 'test') return { text: 'Testnet (test coins)', test: true };
  if (n === 'main') return { text: 'Mainnet', test: false };
  return { text: 'Not stated in this file', test: false };
}

/* ------------------------------------------------------------ loading */

function parseFragment(): { id: string; token: string; origin: string } | 'incomplete' | null {
  const params = new URLSearchParams(window.location.hash.replace(/^#/, ''));
  const id = params.get('m');
  const token = params.get('r');
  if (!id && !token) return null;
  if (!id || !token) return 'incomplete';
  let origin = window.location.origin;
  const k = params.get('k');
  if (k) {
    // Testing override: only a local keeper is honoured; the token is never
    // sent to any other origin. A refused value falls back to this origin.
    const local = acceptLocalKeeper(k, 'the &k= keeper override');
    if (local !== null) origin = local;
  }
  return { id, token, origin };
}

function setKit(text: string, src: Source): void {
  try {
    kit = parseKit(text);
    source = src;
    problem = null;
    result = null;
  } catch {
    kit = null;
    problem = {
      tone: 'bad',
      title: 'This kit looks damaged',
      body:
        'The file is not a complete Zandglass kit. It may have been cut short or changed on the way. ' +
        'Ask the owner for the claim link or a fresh copy of the file. Nothing was opened.',
    };
  }
}

async function loadMailbox(m: { id: string; token: string; origin: string }): Promise<void> {
  loading = true;
  problem = null;
  render();
  try {
    const res = await fetch(`${m.origin}/v1/mailboxes/${encodeURIComponent(m.id)}/kit`, {
      headers: { Authorization: `Bearer ${m.token}` },
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
    });
    if (res.status === 404) {
      problem = {
        tone: 'bad',
        title: 'No kit found for this link',
        body:
          'The link may be mistyped, or the owner removed the mailbox. Ask the owner to send the claim link again. ' +
          'If you have a kit file, you can open it below.',
      };
    } else if (res.status === 401 || res.status === 403) {
      problem = {
        tone: 'bad',
        title: 'This link is not accepted',
        body: 'The link is incomplete or has been replaced. Ask the owner for the latest claim link.',
      };
    } else if (!res.ok) {
      throw new NetworkError(`keeper answered ${res.status}`);
    } else {
      const text = await res.text();
      if (text.length > MAX_KIT_BYTES) throw new KitDamagedError();
      setKit(text, { kind: 'mailbox', ...m });
    }
  } catch (e) {
    if (e instanceof KitDamagedError) {
      problem = { tone: 'bad', title: 'This kit looks damaged', body: 'The kit is too large to be a real one. Nothing was opened.' };
    } else {
      problem = {
        tone: 'warn',
        title: 'Could not reach the kit server',
        body: 'Check your internet connection and try again. Your claim link is fine.',
        retry: () => void loadMailbox(m),
      };
    }
  }
  loading = false;
  render();
}

async function loadFile(file: File): Promise<void> {
  if (file.size > MAX_KIT_BYTES) {
    kit = null;
    problem = { tone: 'bad', title: 'This kit looks damaged', body: 'That file is too large to be a Zandglass kit.' };
    render();
    return;
  }
  setKit(await file.text(), { kind: 'file', name: file.name });
  render();
}

/* ------------------------------------------------------------ opening */

async function open(): Promise<void> {
  if (!kit || opening) return;
  const words = phraseEl.value;
  if (!isValidPhrase(words)) {
    problem = {
      tone: 'warn',
      title: 'That does not look like a full heir phrase',
      body: 'Enter all 24 words, in order, separated by spaces. Check the spelling against your paper.',
    };
    render();
    phraseEl.focus();
    return;
  }
  opening = true;
  problem = null;
  render();
  try {
    result = await openKit(kit, words, relayClient());
    phraseEl.value = '';
    seedShown = false;
  } catch (e) {
    result = null;
    if (e instanceof WrongHeirKeyError) {
      problem = {
        tone: 'bad',
        title: 'These words do not open this kit',
        body:
          'The kit was sealed for a different heir key. Check that you used the words you wrote down when you created your heir key, ' +
          'and that the owner used your heir code. You can try again as often as you like.',
      };
    } else if (e instanceof InvalidPhraseError) {
      problem = { tone: 'warn', title: 'That does not look like a full heir phrase', body: 'Enter all 24 words, in order.' };
    } else if (e instanceof BeaconNotReadyError) {
      problem = {
        tone: 'info',
        title: 'The time key is not published yet',
        body: 'The unlock time has just passed and the drand network is about to publish the key. Try again in a few seconds.',
        retry: () => void open(),
      };
    } else if (e instanceof NetworkError) {
      problem = {
        tone: 'warn',
        title: 'Could not reach the drand time servers',
        body: 'Check your internet connection and try again. Your words were not sent anywhere.',
        retry: () => void open(),
      };
    } else {
      problem = {
        tone: 'bad',
        title: 'This kit looks damaged',
        body:
          'The time lock opened, but what is inside could not be read. The file may have been changed or cut short. ' +
          'Ask the owner for the claim link or a fresh copy.',
      };
    }
  }
  opening = false;
  render();
}

/* ------------------------------------------------------------ views */

function problemView(p: Problem): HTMLElement {
  return h(
    'div',
    { class: `notice ${p.tone}`, role: p.tone === 'bad' ? 'alert' : 'status' },
    h(
      'div',
      {},
      h('strong', {}, p.title),
      h('p', {}, p.body),
      p.retry
        ? h('button', { type: 'button', class: 'btn', onclick: p.retry }, 'Try again')
        : null,
    ),
  );
}

function loaderView(): HTMLElement {
  const input = h('input', {
    type: 'file',
    id: 'kit-file',
    accept: '.zandglass,.json,.txt,application/json,text/plain',
    onchange: (e: Event) => {
      const f = (e.target as HTMLInputElement).files?.[0];
      if (f) void loadFile(f);
    },
  });
  const drop = h(
    'label',
    { class: 'drop', for: 'kit-file' },
    h('span', { class: 'label' }, 'Choose your kit file'),
    h('span', { class: 'muted small' }, ' or drop it here. It ends in .zandglass.'),
    h('br'),
    input,
  );
  drop.addEventListener('dragover', (e) => {
    e.preventDefault();
    drop.classList.add('over');
  });
  drop.addEventListener('dragleave', () => drop.classList.remove('over'));
  drop.addEventListener('drop', (e) => {
    e.preventDefault();
    drop.classList.remove('over');
    const f = e.dataTransfer?.files[0];
    if (f) void loadFile(f);
  });
  return h(
    'section',
    { class: 'card' },
    h('h2', {}, 'Open your kit'),
    h(
      'p',
      {},
      'The owner gave you either a claim link or a kit file. A claim link opens by itself. If you have a file, pick it below. ',
      'The file never leaves your device.',
    ),
    drop,
  );
}

function kitView(k: LoadedKit): HTMLElement {
  const unlocked = isUnlocked(k);
  const t = formatUnlock(unlockSeconds(k));
  const net = networkLabel(k);
  const facts = h(
    'dl',
    { class: 'facts' },
    h('div', {}, h('dt', {}, 'Network'), h('dd', {}, h('span', { class: net.test ? 'badge test' : 'badge' }, net.text))),
    h('div', {}, h('dt', {}, 'Vault number'), h('dd', {}, k.meta?.epoch !== null && k.meta?.epoch !== undefined ? `#${k.meta.epoch}` : 'Not stated')),
    h('div', {}, h('dt', {}, 'Unlock time'), h('dd', {}, t.local, h('br'), h('span', { class: 'muted small' }, t.utc))),
    h('div', {}, h('dt', {}, 'Time key round'), h('dd', {}, String(k.round))),
  );
  const fileHint =
    source?.kind === 'file'
      ? h(
          'div',
          { class: 'notice info' },
          h(
            'div',
            {},
            h('strong', {}, 'Is this the latest kit?'),
            h(
              'p',
              {},
              'A file can be an older copy. The owner may have moved the funds to a newer vault since it was made. ' +
                'The claim link always serves the latest kit, so ask the owner for it if you can.',
            ),
          ),
        )
      : null;
  updateCountdown();
  return h(
    'section',
    { class: 'card', 'aria-labelledby': 'kit-title' },
    h(
      'div',
      { class: 'kit-head' },
      h(
        'div',
        {},
        h('h2', { id: 'kit-title' }, unlocked ? 'This kit is unlocked' : 'This kit is still locked'),
        countdownEl,
        lateTimeEl,
      ),
    ),
    facts,
    fileHint,
    unlocked
      ? null
      : h(
          'p',
          { class: 'muted' },
          'Nobody can open this kit before the unlock time, not even you, the owner or us. ',
          'Come back then. This page will update by itself if you leave it open.',
        ),
  );
}

function updateCountdown(): void {
  if (!kit) return;
  const left = unlockSeconds(kit) - nowSeconds();
  if (left > 0) {
    countdownEl.textContent = `Unlocks in ${formatLeft(left)}`;
  } else {
    countdownEl.textContent = 'Ready to open';
  }
}

function tick(): void {
  if (!kit) return;
  const wasLocked = countdownEl.textContent?.startsWith('Unlocks') ?? false;
  updateCountdown();
  paintStage();
  if (wasLocked && isUnlocked(kit)) {
    lateTimeEl.textContent = 'The kit is now unlocked.';
    render();
  }
}

function openView(): HTMLElement {
  return h(
    'section',
    { class: 'card' },
    h('h2', {}, 'Enter your heir phrase'),
    h(
      'p',
      { id: 'phrase-help' },
      'Type the 24 words you wrote down when you created your heir key. ',
      'They are used inside this page only. They are not saved and not sent.',
    ),
    h(
      'form',
      {
        novalidate: true,
        onsubmit: (e: Event) => {
          e.preventDefault();
          void open();
        },
      },
      h('label', { for: 'phrase' }, 'Your 24 words'),
      phraseEl,
      h(
        'div',
        { class: 'actions' },
        h(
          'button',
          { type: 'submit', class: 'btn-primary', disabled: opening ? true : null },
          opening ? 'Opening...' : 'Open the kit',
        ),
      ),
      opening
        ? h(
            'p',
            { class: 'muted', role: 'status' },
            h('span', { class: 'spinner' }),
            ' Fetching the time key from drand and opening both layers here in your browser.',
          )
        : null,
    ),
  );
}

const WALLETS: { name: string; steps: string[] }[] = [
  {
    name: 'Zodl',
    steps: [
      'Install Zodl on your phone and choose to restore an existing wallet.',
      'Enter the 24 words of the vault seed, in order.',
      'When asked for a birthday or restore height, enter the birthday height shown above. Then let it sync.',
    ],
  },
  {
    name: 'Zkool',
    steps: [
      'Open Zkool and add a new account from an existing seed phrase.',
      'Paste the 24 words of the vault seed.',
      'Set the birthday height shown above so it does not scan the whole chain, then sync.',
    ],
  },
  {
    name: 'Vizor',
    steps: [
      'Open Vizor and choose to restore a wallet from a seed phrase.',
      'Enter the 24 words of the vault seed.',
      'Enter the birthday height shown above if it asks for one, then sync.',
    ],
  },
  {
    name: 'Zingo',
    steps: [
      'Open Zingo and choose to restore from a seed phrase.',
      'Enter the 24 words of the vault seed and the birthday height shown above.',
      'Let it sync. Your balance appears when it finishes.',
    ],
  },
  {
    name: 'Zandglass command line',
    steps: [
      'Save your kit file, or use the claim link, then run: zandglass claim --to <your Zcash address>.',
      'Enter your heir phrase when asked. The funds are sent to your address in an ordinary shielded transaction.',
    ],
  },
];

function resultView(r: OpenedPayload): HTMLElement {
  const seed = r.kind === 'json' ? r.payload.vault_seed : r.text;
  const seedBox = h('div', { class: 'seed hidden', id: 'seed', 'aria-live': 'polite' });
  const toggle = h('button', { type: 'button', class: 'btn', 'aria-pressed': 'false' }, 'Show the seed');
  const paint = (): void => {
    seedBox.className = seedShown ? 'seed' : 'seed hidden';
    seedBox.textContent = seedShown
      ? seed
      : r.kind === 'json'
        ? r.payload.vault_seed.split(/\s+/).map(() => '••••').join(' ')
        : '•••• •••• ••••';
    toggle.textContent = seedShown ? 'Hide the seed' : 'Show the seed';
    toggle.setAttribute('aria-pressed', String(seedShown));
  };
  toggle.addEventListener('click', () => {
    seedShown = !seedShown;
    paint();
  });
  paint();

  const payload = r.kind === 'json' ? r.payload : null;
  const hints = payload?.restore_hints ?? [];
  const parts: Child[] = [
    h('h2', { tabindex: '-1', id: 'result-title' }, 'Your inheritance is open'),
    h(
      'div',
      { class: 'notice ok', role: 'status' },
      h('div', {}, h('strong', {}, 'The kit opened'), h('p', {}, 'Restore the seed below in a Zcash wallet to see the funds. Make sure nobody is watching your screen.')),
    ),
  ];
  if (payload?.message) {
    parts.push(h('h3', {}, 'A message from the owner'), h('blockquote', { class: 'message' }, payload.message));
  }
  parts.push(
    h('h3', {}, r.kind === 'json' ? 'Vault seed (24 words)' : 'Contents'),
    seedBox,
    h('div', { class: 'actions' }, toggle, copyButton('Copy the seed', () => seed)),
  );
  if (payload) {
    parts.push(
      h('h3', {}, 'Birthday height'),
      h('p', { class: 'kv' }, payload.birthday_height.toLocaleString('en-US')),
      h('p', { class: 'muted small' }, 'Enter this when you restore, so the wallet only scans from this block on.'),
    );
  }
  parts.push(
    h('h3', {}, 'Restore it in a wallet'),
    h(
      'p',
      { class: 'muted' },
      payload?.network === 'test'
        ? 'This is a testnet vault. Use a wallet set to testnet.'
        : 'Pick any wallet that supports Zcash shielded funds.',
    ),
    ...WALLETS.map((w, i) =>
      h(
        'details',
        { open: i === 0 ? true : null },
        h('summary', {}, w.name),
        h('div', {}, h('ol', { class: 'checklist' }, ...w.steps.map((s) => h('li', {}, s)))),
      ),
    ),
  );
  if (hints.length) {
    parts.push(h('h3', {}, 'Notes from the owner'), h('ul', { class: 'checklist' }, ...hints.map((x) => h('li', {}, x))));
  }
  parts.push(
    h(
      'div',
      { class: 'notice info' },
      h(
        'div',
        {},
        h('strong', {}, 'Wallet looks empty?'),
        h(
          'p',
          {},
          'This vault may have been moved. The owner refreshes the vault from time to time and sends the funds to a new one. ',
          'If the balance is zero, get the latest kit from your claim link and open that one. ',
          'If you only have a file, ask the owner for the link.',
        ),
      ),
    ),
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        {
          type: 'button',
          class: 'btn',
          onclick: () => {
            result = null;
            seedShown = false;
            phraseEl.value = '';
            render();
          },
        },
        'Clear this from the screen',
      ),
    ),
  );
  return h('section', { class: 'card' }, ...parts);
}

function render(): void {
  clear(main);
  const flow = h(
    'div',
    { class: 'app-flow' },
    h('span', { class: 'eyebrow' }, 'For the heir, when the time comes'),
    h('h1', {}, 'Claim your ', h('em', {}, 'inheritance')),
    h(
      'p',
      { class: 'lead' },
      'Open a Zandglass kit here. It works entirely in your browser, and your words and keys never leave it.',
    ),
  );
  main.append(h('div', { class: 'app-console' }, stage, flow));
  paintStage();
  if (problem) flow.append(problemView(problem));
  if (loading) {
    flow.append(
      h('div', { class: 'card', role: 'status' }, h('span', { class: 'spinner' }), ' Loading your kit...'),
    );
    return;
  }
  if (!kit) {
    flow.append(loaderView());
    return;
  }
  flow.append(kitView(kit));
  if (result) {
    flow.append(resultView(result));
    document.getElementById('result-title')?.focus();
  } else if (isUnlocked(kit)) {
    flow.append(openView());
  }
  if (source?.kind === 'file' || !isUnlocked(kit) || result) {
    flow.append(
      h(
        'p',
        {},
        h(
          'button',
          {
            type: 'button',
            class: 'btn-quiet',
            onclick: () => {
              kit = null;
              source = null;
              result = null;
              problem = null;
              phraseEl.value = '';
              render();
            },
          },
          'Open a different kit',
        ),
      ),
    );
  }
}

/* ------------------------------------------------------------ start */

window.clearInterval(timer);
timer = window.setInterval(tick, 1000);

const frag = parseFragment();
if (frag === 'incomplete') {
  problem = {
    tone: 'bad',
    title: 'This claim link is incomplete',
    body: 'Part of the link is missing. Copy the whole link from the message the owner sent you, or open a kit file below.',
  };
  render();
} else if (frag) {
  void loadMailbox(frag);
} else {
  render();
}

