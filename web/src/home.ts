import './style.css';
import './site.css';
import './scenes.css';
import './landing.css';
import { h } from './lib/dom';
import { GlassCanvas, drandTicker, mountDust, reducedMotion, revealOnScroll } from './lib/fx';
import { formatCountdown, networkLabel, parseLiveVault, type LiveVault } from './lib/playground';
import { sceneFlip, sceneHeir, sceneOpen, sceneSeal } from './lib/scenes';
import { mountShell } from './lib/shell';

const REPO = 'https://github.com/0xBadSign/zandglass';
const EVIDENCE = `${REPO}/blob/main/docs/evidence/EVIDENCE.md`;

const main = mountShell('home', { wide: true });
mountDust();

/* ------------------------------------------------------------------ hero */

const glass = new GlassCanvas({ autoplay: true, period: 13, className: 'hero-glass' });

const hero = h(
  'section',
  { class: 'hero3' },
  h(
    'div',
    { class: 'hero-copy' },
    h('span', { class: 'eyebrow' }, 'Time-locked inheritance for shielded Zcash'),
    h('h1', { class: 'mega' }, h('span', { class: 'l1' }, 'Pass on your'), h('span', { class: 'l2' }, 'shielded ZEC.'), h('em', { class: 'l3' }, 'Privately.')),
    h(
      'p',
      { class: 'lead' },
      'Your heir can open your vault only after you stop checking in. Until then nobody can, and nothing on chain reveals the plan, the heir or the amounts.',
    ),
    h(
      'div',
      { class: 'actions' },
      h('a', { class: 'btn-primary btn-lg', href: './playground' }, 'Try the playground', h('span', { class: 'arrow', 'aria-hidden': 'true' }, '→')),
      h('a', { class: 'btn btn-lg ghost', href: './heir' }, 'Become an heir'),
    ),
    h('div', { class: 'hero-meta' }, drandTicker(), h('span', { class: 'hm-note' }, 'The clock every kit is sealed to. One tick every 3 seconds.')),
  ),
  h('div', { class: 'hero-stage' }, h('div', { class: 'halo', 'aria-hidden': 'true' }), glass.el),
  h('a', { class: 'scroll-cue', href: '#problem', 'aria-label': 'Scroll to the story' }, h('span', {})),
);

/* --------------------------------------------------------------- marquee */

const TAGS = ['No custody', 'No smart contract', 'No plan on chain', 'Nothing pre-signed', 'Open source', 'Restores in any wallet'];
const marquee = h(
  'div',
  { class: 'marquee', 'aria-hidden': 'true' },
  h('div', { class: 'mq-track' }, ...[0, 1].flatMap(() => TAGS.map((t) => h('span', {}, t)))),
);

/* --------------------------------------------------------------- problem */

const problem = h(
  'section',
  { class: 'problem reveal', id: 'problem', 'aria-labelledby': 'problem-h' },
  h('p', { class: 'kicker' }, 'The problem'),
  h(
    'h2',
    { id: 'problem-h', class: 'statement' },
    'Shielded Zcash has ',
    h('em', {}, 'no timelocks.'),
    ' If you die, your heirs cannot even see that the funds ',
    h('span', { class: 'ghost-word' }, 'exist.'),
  ),
  h(
    'p',
    { class: 'statement-sub' },
    'Writing the seed in a will gives it to a lawyer today. Telling your heir gives it to them today. Zandglass gives it to them ',
    h('em', {}, 'later'),
    ', and only if you go silent.',
  ),
);

/* ------------------------------------------------------- how it works */

const STEPS = [
  {
    n: '01',
    title: 'Your heir makes a key',
    body: 'In their own browser: 24 words that never leave them, and a short public heir code they send to you.',
    scene: sceneHeir,
  },
  {
    n: '02',
    title: 'Zandglass seals a vault',
    body: 'Your funds move into a fresh vault. Its seed is locked twice: to a future drand round, and to the heir code. You hold the kit.',
    scene: sceneSeal,
  },
  {
    n: '03',
    title: 'You flip, now and then',
    body: 'A flip moves the funds to a new vault and seals a new kit with a new timer. Every older kit now opens an empty vault.',
    scene: sceneFlip,
  },
  {
    n: '04',
    title: 'The sand runs out',
    body: 'If you stop flipping, the round arrives. Your heir opens the kit in a browser and restores the vault in any Zcash wallet.',
    scene: sceneOpen,
  },
];

const stageScenes = STEPS.map((s, i) => h('div', { class: `stage-scene${i === 0 ? ' on' : ''}`, 'data-i': String(i) }, s.scene()));
const stepEls = STEPS.map((s, i) =>
  h(
    'li',
    { class: 'story-step', 'data-i': String(i) },
    h('div', { class: 'inline-scene', 'aria-hidden': 'true' }, s.scene()),
    h('span', { class: 'step-n' }, s.n),
    h('h3', {}, s.title),
    h('p', {}, s.body),
  ),
);
const progressFill = h('span', {});
const how = h(
  'section',
  { class: 'story', 'aria-labelledby': 'how' },
  h('p', { class: 'kicker' }, 'How it works'),
  h('h2', { id: 'how', class: 'big-title' }, 'Four moves. ', h('em', {}, 'No contract.')),
  h(
    'div',
    { class: 'story-grid' },
    h('div', { class: 'story-stage', 'aria-hidden': 'true' }, h('div', { class: 'stage-frame' }, ...stageScenes), h('div', { class: 'stage-progress' }, progressFill)),
    h('ol', { class: 'story-steps' }, ...stepEls),
  ),
);

function wireStory(): void {
  const set = (i: number): void => {
    stageScenes.forEach((s, j) => s.classList.toggle('on', i === j));
    stepEls.forEach((s, j) => s.classList.toggle('on', i === j));
    progressFill.style.transform = `scaleY(${((i + 1) / STEPS.length).toFixed(3)})`;
  };
  set(0);
  if (!('IntersectionObserver' in window)) return;
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) if (e.isIntersecting) set(Number((e.target as HTMLElement).dataset.i));
    },
    { rootMargin: '-45% 0px -45% 0px' },
  );
  stepEls.forEach((s) => io.observe(s));
}

/* ------------------------------------------------- spot the inheritance */

function hex(n: number, seed: number): string {
  let s = seed;
  let out = '';
  for (let i = 0; i < n; i++) {
    s = (s * 1664525 + 1013904223) >>> 0;
    out += '0123456789abcdef'[s >>> 28];
  }
  return out;
}

const TILE_COUNT = 24;
const spotMsg = h('p', { class: 'spot-msg', 'aria-live': 'polite' }, 'Pick the one you think is an inheritance.');
let picks = 0;
const tiles = Array.from({ length: TILE_COUNT }, (_, i) => {
  const id = hex(8, i * 7919 + 17);
  const actions = i % 4 === 1 ? 3 : 2;
  const b = h(
    'button',
    { type: 'button', class: 'tx-tile', 'aria-label': `Shielded transaction ${id}` },
    h(
      'span',
      { class: 'tt-front' },
      h('span', { class: 'tt-lock', 'aria-hidden': 'true' }),
      h('span', { class: 'tt-id' }, `${id}…`),
      h('span', { class: 'tt-row' }, `${actions} actions`),
      h('span', { class: 'tt-row' }, 'fee 0.0001'),
    ),
    h('span', { class: 'tt-back' }, 'Ordinary', h('br', {}), 'shielded'),
  );
  b.addEventListener('click', () => {
    if (b.classList.contains('flipped')) return;
    b.classList.add('flipped');
    picks++;
    spotMsg.textContent =
      picks === 1
        ? 'Ordinary shielded transaction. Try another.'
        : picks === 2
          ? 'Ordinary again. A Zandglass flip uses the standard fee, expiry and anchor of any wallet.'
          : 'You cannot tell, and neither can anyone watching the chain. That is the point.';
    if (picks >= 3) spot.classList.add('solved');
  });
  return b;
});
const spot = h(
  'section',
  { class: 'spot reveal', 'aria-labelledby': 'spot-h' },
  h(
    'div',
    { class: 'spot-copy' },
    h('p', { class: 'kicker' }, 'Privacy'),
    h('h2', { id: 'spot-h', class: 'big-title' }, 'Spot the ', h('em', {}, 'inheritance.')),
    h(
      'p',
      { class: 'muted' },
      'A block of shielded transactions (an illustration). On the real chain, Zandglass funding, flips and claims are built by the standard Zcash wallet library at send time: nothing is scheduled, pre-signed or marked.',
    ),
    spotMsg,
  ),
  h('div', { class: 'spot-grid' }, ...tiles),
);

/* ----------------------------------------------------------------- proof */

const PROOF: { step: string; what: string; txid: string }[] = [
  { step: 'Fund', what: 'The owner funds a fresh vault', txid: 'b58f467f642d9e7a777f6d84f73e6dbd8fea3cf2fd51f3f526d3a5c2d29adc01' },
  { step: 'Flip', what: 'Funds roll to a new vault, a new kit is sealed', txid: 'd5ce33a9c4dc04a5b12609cf2865d0100af3c2aae4ae5f1f570d8482ea5cebfe' },
  { step: 'Claim', what: 'The owner went silent; the heir opened the kit', txid: 'e4fb38b61ac5a81623701809b29e1ff50adedf1bb441fa67e573495aed01a88c' },
];
const shortId = (t: string): string => `${t.slice(0, 10)}…${t.slice(-8)}`;

const proof = h(
  'section',
  { class: 'chain reveal', 'aria-labelledby': 'proof' },
  h('p', { class: 'kicker' }, 'Evidence'),
  h('h2', { id: 'proof', class: 'big-title' }, 'Already ', h('em', {}, 'on chain.')),
  h('p', { class: 'muted narrow' }, 'A full inheritance, end to end on Zcash mainnet, plus testnet runs of scheduled payments and of a vault restored and spent in a third-party wallet. Each step is an ordinary shielded transaction.'),
  h(
    'ol',
    { class: 'chain-line' },
    ...PROOF.map((p, i) =>
      h(
        'li',
        { class: 'block' },
        h('span', { class: 'blk-cube', 'aria-hidden': 'true' }, h('span', {}), h('span', {}), h('span', {})),
        h('span', { class: 'blk-n' }, `0${i + 1}`),
        h('span', { class: 'blk-step' }, p.step),
        h('span', { class: 'blk-what' }, p.what),
        h('code', { class: 'blk-tx', title: p.txid }, shortId(p.txid)),
      ),
    ),
  ),
  h('a', { class: 'link-arrow', href: EVIDENCE }, 'All transaction ids and how to check them', h('span', { 'aria-hidden': 'true' }, ' →')),
);

/* ------------------------------------------------------------ who sees */

const LEAKS: { icon: string; who: string; learns: string; not: string }[] = [
  { icon: 'eye', who: 'Anyone watching the chain', learns: 'Ordinary shielded transactions, like any wallet’s.', not: 'That a plan exists, the heir, the amounts, the unlock time, or any link between vaults.' },
  { icon: 'clock', who: 'The drand network', learns: 'At most, that someone fetched a public time key.', not: 'Anything about the kit.' },
  { icon: 'box', who: 'The mailbox keeper', learns: 'That a mailbox exists, when its kit changes, the unlock round.', not: 'Owner, heir, amounts, addresses or contents. It can never decrypt.' },
  { icon: 'heir', who: 'The heir, after the unlock', learns: 'The current vault: its balance and its one incoming transfer.', not: 'Earlier vaults, or the owner’s other funds.' },
  { icon: 'mask', who: 'Someone who steals a kit', learns: 'Nothing, before or after the unlock.', not: 'It cannot be opened without the heir key.' },
];

const sees = h(
  'section',
  { class: 'sees', 'aria-labelledby': 'leak' },
  h('p', { class: 'kicker' }, 'Threat model'),
  h('h2', { id: 'leak', class: 'big-title' }, 'Who sees ', h('em', {}, 'what.')),
  h(
    'ul',
    { class: 'sees-grid' },
    ...LEAKS.map((l) =>
      h(
        'li',
        { class: 'sees-card reveal' },
        h('span', { class: `sees-icon i-${l.icon}`, 'aria-hidden': 'true' }),
        h('h3', {}, l.who),
        h('p', { class: 'sees-row' }, h('span', { class: 'sees-k' }, 'Sees'), l.learns),
        h('p', { class: 'sees-row no' }, h('span', { class: 'sees-k' }, 'Never sees'), h('span', { class: 'redacted' }, l.not)),
      ),
    ),
  ),
);

/* -------------------------------------------------------------- honesty */

const REAL = [
  'Zcash transactions for funding, flips, installments and claims, with transaction ids.',
  'drand quicknet time-lock encryption, and age X25519 heir keys.',
  'A blind mailbox that cannot read what it stores.',
  'A vault restored and spent in ECC’s zcash-devtool; kits opened with the reference tle and age tools.',
  'An owner panel on your own computer (zandglass ui) and calendar reminders that no server sees.',
];
const MOCK = [
  'In the playground the server plays the owner and holds a tiny demo vault. Your heir key is real and stays in your browser.',
  'Playground timers are minutes instead of months.',
  'The code is unaudited, and mainnet plans are capped by default.',
  'Installment plans are created from the command line; the owner panel shows and cancels them.',
];
const honesty = h(
  'section',
  { class: 'honesty reveal', 'aria-labelledby': 'real' },
  h('p', { class: 'kicker' }, 'Honesty'),
  h('h2', { id: 'real', class: 'big-title' }, 'What is real, ', h('em', {}, 'what is a mock.')),
  h(
    'div',
    { class: 'hon-cols' },
    h('div', { class: 'hon real' }, h('h3', {}, 'Real'), h('ul', {}, ...REAL.map((t) => h('li', {}, t)))),
    h('div', { class: 'hon mock' }, h('h3', {}, 'Mock or limited'), h('ul', {}, ...MOCK.map((t) => h('li', {}, t)))),
  ),
);

/* ---------------------------------------------------------------- final */

const finalGlass = new GlassCanvas({ autoplay: true, period: 9, className: 'final-glass' });
const final = h(
  'section',
  { class: 'final reveal', 'aria-labelledby': 'final-h' },
  h('div', { class: 'final-art' }, finalGlass.el),
  h('h2', { id: 'final-h', class: 'mega final-title' }, 'Turn the ', h('em', {}, 'glass.')),
  h('p', { class: 'lead' }, 'Five minutes on a real network. You play the heir; the server plays the owner.'),
  h(
    'div',
    { class: 'actions center' },
    h('a', { class: 'btn-primary btn-lg', href: './playground' }, 'Open the playground', h('span', { class: 'arrow', 'aria-hidden': 'true' }, '→')),
  ),
  h(
    'ul',
    { class: 'final-links' },
    h('li', {}, h('a', { href: './heir' }, 'Become an heir')),
    h('li', {}, h('a', { href: './claim' }, 'Claim a kit')),
    h('li', {}, h('a', { href: './owner' }, 'Owner panel')),
    h('li', {}, h('a', { href: EVIDENCE }, 'Evidence')),
    h('li', {}, h('a', { href: REPO }, 'Source code')),
    h('li', {}, h('a', { href: 'https://x.com/ZandGlass', rel: 'noopener noreferrer' }, 'Updates on X')),
  ),
);

main.append(hero, marquee, h('div', { id: 'live-slot' }), problem, how, spot, proof, sees, honesty, final);
wireStory();
revealOnScroll();
if (!reducedMotion()) document.body.classList.add('motion');

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
