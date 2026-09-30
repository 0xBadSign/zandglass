import './style.css';
import qrcode from 'qrcode-generator';
import { clear, copyButton, h, svg } from './lib/dom';
import { generatePhrase, heirCodeFromPhrase, normalizePhrase, pickPositions } from './lib/heir';
import { mountShell } from './lib/shell';

type Step = 'intro' | 'backup' | 'confirm' | 'code';
const STEPS: { id: Step; label: string }[] = [
  { id: 'intro', label: 'Start' },
  { id: 'backup', label: 'Back up' },
  { id: 'confirm', label: 'Check' },
  { id: 'code', label: 'Share code' },
];

const main = mountShell('heir');

// The phrase lives only in this closure. It is never stored or sent.
let phrase = '';
let positions: number[] = [];
let heirCode = '';
let step: Step = 'intro';
let confirmError = '';

function qrSvg(text: string): SVGElement {
  const qr = qrcode(0, 'M');
  qr.addData(text);
  qr.make();
  const n = qr.getModuleCount();
  let d = '';
  for (let r = 0; r < n; r++) {
    for (let c = 0; c < n; c++) {
      if (qr.isDark(r, c)) d += `M${c + 1} ${r + 1}h1v1h-1z`;
    }
  }
  const s = svg('svg', {
    viewBox: `0 0 ${n + 2} ${n + 2}`,
    role: 'img',
    'aria-label': 'QR code of your heir code',
    'shape-rendering': 'crispEdges',
  });
  s.append(svg('path', { d, fill: '#000' }));
  return s;
}

function stepper(): HTMLElement {
  const idx = STEPS.findIndex((s) => s.id === step);
  return h(
    'ol',
    { class: 'steps', 'aria-label': 'Progress' },
    ...STEPS.map((s, i) =>
      h(
        'li',
        { class: i < idx ? 'done' : '', 'aria-current': i === idx ? 'step' : null },
        `${i + 1}. ${s.label}`,
      ),
    ),
  );
}

function go(next: Step): void {
  step = next;
  render();
  const heading = main.querySelector<HTMLElement>('h2');
  heading?.focus();
  window.scrollTo({ top: 0 });
}

function reset(): void {
  phrase = '';
  positions = [];
  heirCode = '';
  confirmError = '';
  go('intro');
}

function intro(): HTMLElement {
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, 'Create your heir key'),
    h(
      'p',
      {},
      'Your heir key is 24 words. The words stay with you. From them we make a short public ' +
        'heir code that you send to the person who is naming you as their heir.',
    ),
    h(
      'ul',
      { class: 'checklist' },
      h('li', {}, 'It takes about two minutes. You will need a pen and paper.'),
      h('li', {}, 'The words are created here, in your browser, and are never sent anywhere.'),
      h('li', {}, 'Nothing is saved. If you close this page, the words are gone unless you wrote them down.'),
    ),
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
            positions = pickPositions(3, 24);
            go('backup');
          },
        },
        'Create my words',
      ),
    ),
  );
}

function backup(): HTMLElement {
  const words = phrase.split(' ');
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, 'Write down your 24 words'),
    h(
      'p',
      {},
      'Write them on paper, in this order. Keep the paper somewhere safe, such as a locked drawer. ',
      'These words are the only way to open what you will inherit.',
    ),
    h('ol', { class: 'phrase', 'aria-label': 'Your 24 words' }, ...words.map((w) => h('li', {}, w))),
    h(
      'div',
      { class: 'notice warn', role: 'note' },
      h(
        'div',
        {},
        h('strong', {}, 'Keep these words secret'),
        h(
          'p',
          {},
          'Never send them to the owner, to us, or to anyone else. Do not take a screenshot or save them in a notes app or cloud drive. ' +
            'Anyone who has the words can claim in your place. If you lose them, nobody can recover them for you.',
        ),
      ),
    ),
    h(
      'div',
      { class: 'actions' },
      h(
        'button',
        { type: 'button', class: 'btn-primary', onclick: () => go('confirm') },
        'I wrote them down',
      ),
      h('button', { type: 'button', class: 'btn-quiet', onclick: reset }, 'Start over'),
    ),
  );
}

function confirm(): HTMLElement {
  const words = phrase.split(' ');
  const inputs = positions.map((p) =>
    h('input', {
      type: 'text',
      id: `w${p}`,
      name: `word-${p}`,
      autocomplete: 'off',
      autocapitalize: 'none',
      autocorrect: 'off',
      spellcheck: 'false',
      inputmode: 'text',
    }),
  );
  const error = h('p', { class: 'field-error', role: 'alert' }, confirmError || null);
  const check = (): void => {
    const ok = positions.every((p, i) => normalizePhrase(inputs[i]?.value ?? '') === words[p - 1]);
    if (!ok) {
      error.textContent = 'One or more words do not match. Check your paper and try again.';
      return;
    }
    confirmError = '';
    void heirCodeFromPhrase(phrase).then((code) => {
      heirCode = code;
      go('code');
    });
  };
  const form = h(
    'form',
    {
      novalidate: true,
      onsubmit: (e: Event) => {
        e.preventDefault();
        check();
      },
    },
    h('div', { class: 'confirm-grid' }, ...positions.map((p, i) =>
      h('div', {}, h('label', { for: `w${p}` }, `Word ${p}`), inputs[i] ?? null),
    )),
    error,
    h(
      'div',
      { class: 'actions' },
      h('button', { type: 'submit', class: 'btn-primary' }, 'Check my words'),
      h('button', { type: 'button', class: 'btn', onclick: () => go('backup') }, 'Show the words again'),
    ),
  );
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, 'Check your paper'),
    h('p', {}, `Type the words at positions ${positions.join(', ')} from your paper. This makes sure your backup is right.`),
    form,
  );
}

function code(): HTMLElement {
  return h(
    'section',
    { class: 'card' },
    h('h2', { tabindex: '-1' }, 'Your heir code'),
    h(
      'div',
      { class: 'notice ok', role: 'status' },
      h('div', {}, h('strong', {}, 'Your key is ready'), h('p', {}, 'This code is public. It is safe to send. It cannot open anything by itself.')),
    ),
    h('code', { class: 'code-box', id: 'heir-code' }, heirCode),
    h(
      'div',
      { class: 'qr-wrap' },
      h('div', { class: 'qr' }, qrSvg(heirCode)),
      h(
        'div',
        {},
        h('div', { class: 'actions' }, copyButton('Copy heir code', () => heirCode, 'btn-primary')),
        h('p', { class: 'muted small' }, 'You can also let the owner scan the QR code.'),
      ),
    ),
    h('h3', {}, 'What to do next'),
    h(
      'ol',
      { class: 'checklist' },
      h('li', {}, 'Send the heir code (the text above, or the QR code) to the owner. A message or email is fine.'),
      h('li', {}, 'Keep your 24 words on paper. You will need them to claim, once the time lock has passed.'),
      h('li', {}, 'The owner will later send you a claim link. Open it on the Claim page when the time comes.'),
    ),
    h(
      'div',
      { class: 'notice warn', role: 'note' },
      h('div', {}, h('strong', {}, 'Send only the code, never the words'), h('p', {}, 'The owner does not need your 24 words and must never receive them.')),
    ),
    h(
      'details',
      {},
      h('summary', {}, 'Show my 24 words again'),
      h('div', {}, h('ol', { class: 'phrase', 'aria-label': 'Your 24 words' }, ...phrase.split(' ').map((w) => h('li', {}, w)))),
    ),
    h(
      'div',
      { class: 'actions' },
      h('a', { class: 'btn', href: './claim.html' }, 'Go to the Claim page'),
      h('button', { type: 'button', class: 'btn-quiet', onclick: reset }, 'Clear and start over'),
    ),
  );
}

function render(): void {
  clear(main);
  main.append(
    h('h1', {}, 'Become an heir'),
    h(
      'p',
      { class: 'lead' },
      'Create a key that lets you receive an inheritance. It is made in your browser and nothing is stored.',
    ),
    stepper(),
    { intro, backup, confirm, code }[step](),
  );
}

render();
