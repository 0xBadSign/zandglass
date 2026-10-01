import './style.css';
import './site.css';
import { h, sandglassArt } from './lib/dom';
import { formatCountdown, networkLabel, parseLiveVault, type LiveVault } from './lib/playground';
import { mountShell } from './lib/shell';

const REPO = 'https://github.com/0xnesQuit/sandglass';
const EVIDENCE = `${REPO}/blob/main/docs/evidence/EVIDENCE.md`;

const main = mountShell('home', { wide: true });

const heroGlass = sandglassArt();

const STEPS: { title: string; body: string }[] = [
  {
    title: 'Heir code',
    body: 'Your heir makes a key in their browser and sends you a short public code. The words stay with them.',
  },
  {
    title: 'Seal',
    body: 'Sandglass moves your funds into a fresh vault and locks its key twice: to a future moment, and to your heir.',
  },
  {
    title: 'Flip',
    body: 'From time to time you flip the sandglass. The funds move to a new vault and the timer starts again. Old kits open an empty vault.',
  },
  {
    title: 'The sand runs out',
    body: 'If you stop flipping, the time passes. Your heir opens the kit in a browser and restores the vault in any Zcash wallet.',
  },
];

const LEAKS: { who: string; learns: string; not: string }[] = [
  {
    who: 'People watching the chain',
    learns: 'Ordinary shielded transactions that look like any wallet.',
    not: 'That a plan exists, who the heir is, amounts, unlock times, or any link between vaults.',
  },
  {
    who: 'The drand network',
    learns: 'At most, that someone fetched a public time key.',
    not: 'Anything about the kit.',
  },
  {
    who: 'The mailbox keeper',
    learns: 'That a mailbox exists, when its kit is replaced, and the unlock round.',
    not: 'Who the owner or heir is, amounts, addresses, or contents. It can never decrypt.',
  },
  {
    who: 'The heir, after the unlock',
    learns: 'The current vault seed, so its balance and its one incoming transfer.',
    not: 'Earlier vaults, or the owner’s other funds.',
  },
  {
    who: 'Someone who steals a kit',
    learns: 'Nothing, before or after the unlock.',
    not: 'It cannot be opened without the heir key.',
  },
];

const REAL = [
  'Zcash mainnet transactions for funding, flips and claims, with evidence transaction ids.',
  'drand quicknet time-lock encryption, and age X25519 heir keys.',
  'A blind mailbox that cannot read what it stores.',
  'Claims restored in third-party wallets, and kits opened with the reference tle and age tools.',
];

const MOCK = [
  'In the playground the server plays the owner and holds a tiny demo vault. Your heir key is real and stays in your browser.',
  'Playground timers are minutes instead of months.',
  'The code is unaudited, and mainnet plans are capped by default.',
  'Reminders are not implemented yet.',
];

function list(items: string[]): HTMLElement {
  return h('ul', { class: 'checklist' }, ...items.map((t) => h('li', {}, t)));
}

const leakTable = h(
  'table',
  { class: 'leaks' },
  h('caption', { class: 'sr-only' }, 'Who learns what'),
  h(
    'thead',
    {},
    h(
      'tr',
      {},
      h('th', { scope: 'col' }, 'Who'),
      h('th', { scope: 'col' }, 'Learns'),
      h('th', { scope: 'col' }, 'Does not learn'),
    ),
  ),
  h(
    'tbody',
    {},
    ...LEAKS.map((r) =>
      h(
        'tr',
        {},
        h('th', { scope: 'row' }, r.who),
        h('td', { 'data-label': 'Learns' }, r.learns),
        h('td', { 'data-label': 'Does not learn' }, r.not),
      ),
    ),
  ),
);

/** Real transactions from docs/evidence/EVIDENCE.md, shown as proof. */
const PROOF: { step: string; what: string; txid: string }[] = [
  { step: 'Fund', what: 'The owner funds a fresh vault', txid: 'c62ad1cbd16bc238ed78349999042db89990cdef992efc408f5d2b55e1fb8d68' },
  { step: 'Flip', what: 'Funds roll to a new vault, a new kit is sealed', txid: 'd14a2d951681f8a53011cde9c6de26755b279c157212c1b8e598de41ce0a1d62' },
  { step: 'Claim', what: 'The owner went silent; the heir opened the kit', txid: '5685741452b15ae0a8f9c458c6fa4fc1b6d6cfe45a3b2c71eebc80604c5a0407' },
];

const shortId = (t: string): string => `${t.slice(0, 10)}…${t.slice(-8)}`;

main.append(
  h(
    'section',
    { class: 'hero2' },
    h(
      'div',
      { class: 'hero-copy' },
      h('span', { class: 'eyebrow' }, 'Time-locked inheritance for Zcash'),
      h('h1', {}, 'Pass on your shielded ZEC. ', h('em', {}, 'Privately.')),
      h(
        'p',
        { class: 'lead' },
        'Shielded Zcash has no timelocks, so there is no private way to pass it on. ' +
          'Sandglass gives your vault a time-locked recovery path: your heir can open it only after you stop checking in ' +
          '— and nothing on chain reveals the plan, the heir or the amounts.',
      ),
      h(
        'div',
        { class: 'actions' },
        h('a', { class: 'btn-primary', href: './playground.html' }, 'Try the playground'),
        h('a', { class: 'btn', href: './heir.html' }, 'Become an heir'),
      ),
      h(
        'ul',
        { class: 'trust' },
        h('li', {}, 'No custody'),
        h('li', {}, 'No smart contract'),
        h('li', {}, 'No plan on chain'),
      ),
    ),
    h('div', { class: 'hero-art' }, heroGlass),
  ),
  h('div', { id: 'live-slot' }),
  h(
    'section',
    { class: 'proof', 'aria-labelledby': 'proof' },
    h(
      'div',
      { class: 'proof-head' },
      h('h2', { id: 'proof' }, 'Already ', h('em', {}, 'on chain.')),
      h(
        'p',
        { class: 'muted' },
        'A full inheritance, end to end on Zcash testnet. Each step is an ordinary shielded transaction: standard expiry, standard fee, no Sandglass-specific fields.',
      ),
    ),
    h(
      'ol',
      { class: 'proof-list' },
      ...PROOF.map((p) =>
        h(
          'li',
          {},
          h('span', { class: 'proof-step' }, p.step),
          h('span', { class: 'proof-what' }, p.what),
          h('code', { class: 'proof-tx', title: p.txid }, shortId(p.txid)),
        ),
      ),
    ),
    h('a', { class: 'proof-more', href: EVIDENCE }, 'See all evidence'),
  ),
  h(
    'section',
    { 'aria-labelledby': 'how' },
    h('h2', { id: 'how', class: 'section-title' }, 'How it works'),
    h(
      'ol',
      { class: 'how' },
      ...STEPS.map((s) => h('li', {}, h('h3', {}, s.title), h('p', {}, s.body))),
    ),
  ),
  h(
    'section',
    { 'aria-labelledby': 'leak' },
    h('h2', { id: 'leak', class: 'section-title' }, 'Why it does not leak'),
    h('p', { class: 'muted' }, 'Every step is an ordinary shielded transaction, built at the moment it is sent. Nothing is scheduled on chain.'),
    leakTable,
  ),
  h(
    'section',
    { 'aria-labelledby': 'real' },
    h('h2', { id: 'real', class: 'section-title' }, 'What is real and what is a mock'),
    h(
      'div',
      { class: 'cols' },
      h('div', { class: 'card' }, h('h3', {}, 'Real'), list(REAL)),
      h('div', { class: 'card' }, h('h3', {}, 'Mock or limited'), list(MOCK)),
    ),
  ),
  h(
    'section',
    { 'aria-labelledby': 'go' },
    h('h2', { id: 'go', class: 'section-title' }, 'Go further'),
    h(
      'ul',
      { class: 'linklist' },
      h('li', {}, h('a', { href: './playground.html' }, 'Try the playground'), ' — a five-minute vault on a real network.'),
      h('li', {}, h('a', { href: './heir.html' }, 'Become an heir'), ' — make your heir key in the browser.'),
      h('li', {}, h('a', { href: './claim.html' }, 'Claim'), ' — open a kit once the time lock has passed.'),
      h('li', {}, h('a', { href: EVIDENCE }, 'Evidence'), ' — transaction ids and reproducible checks.'),
      h('li', {}, h('a', { href: REPO }, 'GitHub repository'), ' — all of the code, open source.'),
    ),
  ),
);

/* ------------------------------------------------------------ live vault */

function liveCard(v: LiveVault): HTMLElement {
  const net = networkLabel(v.network);
  const count = h('p', { class: 'countdown' });
  const note = h('p', { class: 'muted' });
  const paint = (): void => {
    const left = (v.unlockMs - Date.now()) / 1000;
    if (left > 0) {
      count.textContent = formatCountdown(left);
      note.textContent = 'Until the heir can open this vault. No heir or third party can open it earlier.';
    } else {
      count.textContent = v.claimTx ? 'Claimed' : 'The sand has run out';
      note.textContent = v.claimTx
        ? `The heir opened the kit and moved the funds. Transaction ${v.claimTx}`
        : 'The heir can open the kit now.';
    }
  };
  paint();
  window.setInterval(paint, 1000);
  return h(
    'section',
    { class: 'card live-card', 'aria-labelledby': 'live-title' },
    h('h2', { id: 'live-title' }, 'The live vault'),
    h('p', {}, h('span', { class: net.test ? 'badge test' : 'badge' }, net.text)),
    count,
    note,
  );
}

void fetch('./live-vault.json', { cache: 'no-store', credentials: 'omit', referrerPolicy: 'no-referrer' })
  .then((r) => (r.ok ? r.json() : null))
  .then((json: unknown) => {
    const v = parseLiveVault(json);
    if (v) document.getElementById('live-slot')?.append(liveCard(v));
  })
  .catch(() => undefined);
