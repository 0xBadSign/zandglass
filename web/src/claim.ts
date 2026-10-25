import './style.css';
import './site.css';
import './scenes.css';
import './console.css';
import { append, clear, copyButton, h, type Child } from './lib/dom';
import { GlassCanvas, mountDust } from './lib/fx';
import { sceneKit, sceneOpen } from './lib/scenes';
import { BeaconNotReadyError, NetworkError, relayClient } from './lib/drand';
import { InvalidPhraseError, isValidPhrase } from './lib/heir';
import {
  KitDamagedError,
  OwnerSigInvalidError,
  PRACTICE_HANDOFF_KEY,
  checkOwner,
  repinOwnerKey,
  verifyOwnerSig,
  type OwnerCheck,
  WrongHeirKeyError,
  openKit,
  parseKit,
  unlockSeconds,
  type LoadedKit,
  type OpenedPayload,
} from './lib/kit';
import { acceptLocalKeeper } from './lib/local-url';
import { PartAssembler, decodeQrFileAll, hasCameraScan, scanCamera, unlockIcs, type AddResult } from './lib/paper';
import { mountShell } from './lib/shell';

const MAX_KIT_BYTES = 1024 * 1024;

const main = mountShell('claim', { wide: true });
mountDust();

type Source = { kind: 'mailbox'; origin: string; id: string; token: string } | { kind: 'file'; name: string };

type Problem = { tone: 'bad' | 'warn' | 'info'; title: string; body: string; retry?: () => void };

let kit: LoadedKit | null = null;
/** Owner-signature state of the loaded kit: null while checking. */
let owner: OwnerCheck | null = null;
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
    owner = null;
    void checkKitOwner(kit);
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

/** Verify the owner signature and pin the owner key; an invalid signature drops the kit. */
async function checkKitOwner(k: LoadedKit): Promise<void> {
  try {
    const r = await checkOwner(k);
    if (kit !== k) return;
    owner = r;
  } catch (e) {
    if (kit !== k) return;
    kit = null;
    owner = null;
    problem =
      e instanceof OwnerSigInvalidError
        ? {
            tone: 'bad',
            title: 'This kit was altered',
            body:
              'Its owner signature does not match its contents, so someone changed it after the owner sealed it. ' +
              'Do not use it. Ask the owner for the claim link or a fresh copy. Nothing was opened.',
          }
        : { tone: 'bad', title: 'This kit looks damaged', body: 'The owner signature could not be checked. Nothing was opened.' };
  }
  render();
}

/** What the owner signature says about this kit. */
function ownerView(k: LoadedKit): HTMLElement | null {
  if (!owner) return h('p', { class: 'muted small', id: 'owner-sig' }, 'Checking the owner signature...');
  const warn = (title: string, body: string, extra?: HTMLElement) =>
    h('div', { class: 'notice bad', id: 'owner-sig', role: 'alert' }, h('div', {}, h('strong', {}, title), h('p', {}, body), extra ?? null));
  switch (owner.status) {
    case 'match':
      return h('p', { class: 'notice ok', id: 'owner-sig' }, `Signed by the same owner as the kit you opened before (key ${owner.fingerprint}).`);
    case 'new':
      return h(
        'p',
        { class: 'notice info', id: 'owner-sig' },
        `Signed by the owner. Owner key ${owner.fingerprint} is now remembered in this browser; if you can, check it with the owner.`,
      );
    case 'unsigned':
      return h('p', { class: 'muted small', id: 'owner-sig' }, 'This kit carries no owner signature (plans made before signatures, and playground kits, are unsigned).');
    case 'stripped':
      return warn(
        'This kit is not signed, but earlier kits of this plan were',
        `Kits of this plan that this browser saw before were signed by owner key ${owner.pinnedFingerprint}. This one has no signature, so it may be a decoy. Ask the owner before you rely on it.`,
      );
    case 'mismatch': {
      const trust = h(
        'button',
        {
          type: 'button',
          class: 'btn',
          id: 'trust-owner-key',
          onclick: async () => {
            const sig = await verifyOwnerSig(k);
            if (sig !== 'unsigned' && k.meta?.plan) {
              repinOwnerKey(k.meta.plan, sig.key);
              owner = { status: 'match', fingerprint: sig.fingerprint };
              render();
            }
          },
        },
        'The owner confirmed the new key: trust it',
      );
      return warn(
        'This kit was signed by a different owner key',
        `Earlier kits of this plan were signed by ${owner.pinnedFingerprint}; this one by ${owner.fingerprint}. It may be a decoy sealed by someone else. Confirm with the owner before you rely on it.`,
        h('div', { class: 'actions' }, trust),
      );
    }
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
    paperScanView(),
  );
}

/* ------------------------------------------------------------ paper kit */

/** "Scan a paper kit": the camera (BarcodeDetector) or a photo of the QR code. Decoded here; nothing is uploaded. */
const parts = new PartAssembler();

function paperScanView(): HTMLElement {
  const note = h('p', { class: 'muted small', role: 'status', id: 'paper-note' });
  const video = h('video', { class: 'paper-video', playsinline: true, muted: true, 'aria-label': 'Camera view' });
  video.hidden = true;
  let abort: AbortController | null = null;
  const fail = (text: string): void => {
    note.textContent = text;
  };
  const accept = (text: string): void => {
    setKit(text, { kind: 'file', name: 'paper kit' });
    render();
  };
  /** Feed one decoded code. Returns true when the scan is over (kit loaded or rejected). */
  const take = (raw: string): boolean => {
    const r: AddResult = parts.add(raw);
    if (r.status === 'plain' || r.status === 'done') {
      accept(r.text);
      return true;
    }
    if (r.status === 'bad') {
      accept('');
      return true;
    }
    const miss = r.missing.map((i) => `part ${i}`).join(', ');
    note.textContent = `Scanned part ${r.have.length} of ${r.n}. Scan the next one. Still missing: ${miss}.`;
    return false;
  };
  const photo = h('input', {
    type: 'file',
    id: 'paper-photo',
    accept: 'image/*',
    multiple: true,
    onchange: (e: Event) => {
      const input = e.target as HTMLInputElement;
      const files = Array.from(input.files ?? []);
      input.value = '';
      if (!files.length) return;
      note.textContent = 'Reading the QR code...';
      void (async () => {
        let found = 0;
        try {
          for (const f of files) {
            for (const code of await decodeQrFileAll(f)) {
              found++;
              if (take(code)) return;
            }
          }
        } catch {
          fail('That file could not be read as a picture.');
          return;
        }
        if (!found) fail('No QR code found in that picture. Try a sharper, well-lit photo with the whole code in view.');
      })();
    },
  });
  const camBtn = hasCameraScan()
    ? h(
        'button',
        {
          type: 'button',
          class: 'btn',
          id: 'paper-camera',
          onclick: (ev: Event) => {
            const self = ev.currentTarget as HTMLButtonElement;
            if (abort) {
              abort.abort();
              return;
            }
            abort = new AbortController();
            self.textContent = 'Stop the camera';
            video.hidden = false;
            note.textContent = 'Point the camera at the QR code.';
            scanCamera(video, abort.signal, take)
              .catch(() => fail('The camera could not be used. Allow camera access, or choose a photo instead.'))
              .finally(() => {
                abort = null;
                video.hidden = true;
                self.textContent = 'Scan with the camera';
              });
          },
        },
        'Scan with the camera',
      )
    : null;
  return h(
    'div',
    { class: 'paper-scan' },
    h('h3', {}, 'Scan a paper kit'),
    h('p', { class: 'muted small' }, 'If the owner gave you a printed kit, scan every QR code on it (any order) or choose photos of them, several at once if you like. It is read in this browser only.'),
    h('label', { for: 'paper-photo' }, 'Photos of the QR codes'),
    photo,
    camBtn ? h('div', { class: 'actions' }, camBtn) : null,
    video,
    note,
  );
}

/** Heir pulse: when the owner last made a kit, and a calendar file for the unlock. */
function pulseView(k: LoadedKit): HTMLElement | null {
  const created = k.meta?.created ? new Date(k.meta.created) : null;
  const epoch = k.meta?.epoch;
  const parts: Child[] = [];
  if (created && !Number.isNaN(created.getTime())) {
    parts.push(
      h(
        'p',
        { id: 'pulse' },
        `The owner last checked in on ${created.toLocaleDateString(undefined, { dateStyle: 'long' })}`,
        typeof epoch === 'number' ? ` (vault #${epoch}).` : '.',
      ),
      h('p', { class: 'muted small' }, 'If the owner checks in again, a newer kit replaces this one. The claim link always serves the newest.'),
    );
  }
  parts.push(
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        {
          type: 'button',
          class: 'btn',
          id: 'add-calendar',
          onclick: () => {
            const url = URL.createObjectURL(new Blob([unlockIcs(unlockSeconds(k), nowSeconds())], { type: 'text/calendar' }));
            const a = h('a', { href: url, download: 'zandglass-unlock.ics' });
            document.body.append(a);
            a.click();
            a.remove();
            window.setTimeout(() => URL.revokeObjectURL(url), 2000);
          },
        },
        'Add the unlock date to my calendar',
      ),
    ),
  );
  return h('div', { class: 'pulse' }, ...parts);
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
  // A practice kit from the heir page: no "latest kit?" hint and no owner pulse.
  const practice = source?.kind === 'file' && source.name.startsWith('practice-');
  const fileHint =
    source?.kind === 'file' && !practice
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
    practice ? null : ownerView(k),
    fileHint,
    unlocked || practice ? null : pulseView(k),
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

/**
 * A guided restore: pick a wallet, then follow four illustrated steps. The
 * phone drawing shows where each value goes; it never shows the seed words.
 */
function restoreGuide(payload: { birthday_height: number; network?: string } | null): HTMLElement {
  const box = h('div', { class: 'restore-guide' });
  let wallet = 0;
  let step = 0;
  const birthday = payload ? payload.birthday_height.toLocaleString('en-US') : 'the birthday height';
  const STEPS = ['Install', 'Restore', 'Birthday', 'Move the funds'];
  const screen = (k: number): HTMLElement => {
    const w = WALLETS[wallet]?.name ?? 'Wallet';
    if (wallet === WALLETS.length - 1) {
      return h('div', { class: 'ph-term' }, h('span', {}, '$ zandglass claim'), h('span', {}, '  <claim link>'), h('span', {}, '  --phrase-file heir.txt'), h('span', {}, '  --to u1...'), h('span', { class: 'ph-ok' }, 'Moved to your address.'));
    }
    if (k === 0)
      return h('div', { class: 'ph-col' }, h('span', { class: 'ph-logo' }, w.slice(0, 1)), h('strong', {}, w), h('span', { class: 'ph-btn ghost' }, 'Create a new wallet'), h('span', { class: 'ph-btn on' }, 'Restore a wallet'));
    if (k === 1)
      return h(
        'div',
        { class: 'ph-col' },
        h('strong', {}, 'Enter your 24 words'),
        h('div', { class: 'ph-words' }, ...Array.from({ length: 24 }, (_, i) => h('span', {}, String(i + 1)))),
        h('span', { class: 'ph-note' }, 'Type the vault seed, in order'),
      );
    if (k === 2)
      return h(
        'div',
        { class: 'ph-col' },
        h('strong', {}, 'Wallet birthday'),
        h('span', { class: 'ph-field' }, birthday),
        h('span', { class: 'ph-note' }, 'Scanning starts at this block'),
        h('span', { class: 'ph-btn on' }, 'Continue'),
      );
    return h('div', { class: 'ph-col' }, h('strong', {}, 'Send'), h('span', { class: 'ph-field' }, 'To: your own wallet'), h('span', { class: 'ph-field' }, 'Amount: max'), h('span', { class: 'ph-btn on' }, 'Send privately'));
  };
  const textFor = (k: number): string => {
    const w = WALLETS[wallet];
    if (!w) return '';
    if (wallet === WALLETS.length - 1) return w.steps.join(' ');
    return [
      `Install ${w.name}${payload?.network === 'test' ? ' and set it to testnet' : ''}. On the first screen, choose to restore an existing wallet, not to create one.`,
      w.steps[1] ?? 'Enter the 24 words of the vault seed, in order.',
      `When it asks for a birthday or restore height, enter ${birthday}. Then let it sync; the balance appears when it finishes.`,
      'Move everything to a wallet of your own, then forget this vault: the seed was in a kit, so treat it as used.',
    ][k] ?? '';
  };
  const paint = (): void => {
    const cli = wallet === WALLETS.length - 1;
    clear(box);
    append(
      box,
      h(
        'div',
        { class: 'rg-wallets', role: 'group', 'aria-label': 'Wallet' },
        ...WALLETS.map((w, i) =>
          h('button', { type: 'button', class: `rg-wallet${i === wallet ? ' on' : ''}`, 'aria-pressed': String(i === wallet), onclick: () => ((wallet = i), (step = 0), paint()) }, w.name),
        ),
      ),
      cli
        ? null
        : h(
            'ol',
            { class: 'rg-steps', 'aria-label': 'Steps' },
            ...STEPS.map((s, i) => h('li', {}, h('button', { type: 'button', class: i === step ? 'on' : i < step ? 'done' : '', 'aria-current': i === step ? 'step' : null, onclick: () => ((step = i), paint()) }, `${i + 1}. ${s}`))),
          ),
      h(
        'div',
        { class: 'rg-body' },
        h('div', { class: 'phone', 'aria-hidden': 'true' }, h('span', { class: 'phone-notch' }), h('div', { class: 'phone-screen' }, screen(step))),
        h(
          'div',
          { class: 'rg-text' },
          h('p', {}, textFor(step)),
          cli
            ? null
            : h(
                'div',
                { class: 'actions' },
                step > 0 ? h('button', { type: 'button', class: 'btn', onclick: () => ((step -= 1), paint()) }, 'Back') : null,
                step < STEPS.length - 1 ? h('button', { type: 'button', class: 'btn-primary', onclick: () => ((step += 1), paint()) }, 'Next step') : null,
              ),
        ),
      ),
    );
  };
  paint();
  return box;
}

function practiceView(): HTMLElement {
  return h(
    'section',
    { class: 'card', id: 'practice-result' },
    h('h2', { tabindex: '-1', id: 'result-title' }, 'Practice complete'),
    h(
      'div',
      { class: 'notice ok', role: 'status' },
      h(
        'div',
        {},
        h('strong', {}, 'This was a practice kit'),
        h('p', {}, 'No funds. You are ready. Your 24 words opened the seal, so a real kit will open the same way.'),
      ),
    ),
    h('p', {}, 'Keep your words on paper. When the owner sends a real claim link, open it here at the same page.'),
    h('p', { class: 'muted small' }, 'Nothing was stored or sent. There is no seed in a practice kit.'),
  );
}

function resultView(r: OpenedPayload): HTMLElement {
  if (r.kind === 'json' && r.payload.practice) return practiceView();
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
    parts.push(
      h('h3', {}, 'The message in the kit'),
      h('blockquote', { class: 'message' }, payload.message),
      h('p', { class: 'muted small' }, 'Anyone who knows your heir code can make a kit, so treat this message with care: never send your words or money because a message asks for it.'),
    );
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
  parts.push(h('h3', {}, 'Restore it in a wallet'), restoreGuide(payload));
  if (hints.length) {
    parts.push(h('h3', {}, 'Notes in the kit'), h('ul', { class: 'checklist' }, ...hints.map((x) => h('li', {}, x))));
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

// A practice kit handed over from the heir page (same origin, read once).
let handoff: { name: string; text: string } | null = null;
try {
  const raw = sessionStorage.getItem(PRACTICE_HANDOFF_KEY);
  if (raw) {
    sessionStorage.removeItem(PRACTICE_HANDOFF_KEY);
    const v = JSON.parse(raw) as { name?: unknown; text?: unknown };
    if (typeof v.name === 'string' && typeof v.text === 'string') handoff = { name: v.name, text: v.text };
  }
} catch {
  handoff = null;
}

const frag = parseFragment();
if (handoff && !frag) {
  setKit(handoff.text, { kind: 'file', name: handoff.name });
  render();
} else if (frag === 'incomplete') {
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

