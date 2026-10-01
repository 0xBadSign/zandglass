/**
 * Illustrations for "How it works". Constant SVG markup (see svgFrom), animated
 * by classes in scenes.css. Decorative: every scene is aria-hidden and the
 * step text carries the meaning.
 */
import { svgFrom } from './dom';
import { reducedMotion } from './fx';

const NS = 'http://www.w3.org/2000/svg';

const GOLD_DEFS = (p: string): string => `
  <linearGradient id="${p}-gold" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#ffe08a"/><stop offset="0.55" stop-color="#f4b728"/><stop offset="1" stop-color="#b97c0c"/>
  </linearGradient>
  <linearGradient id="${p}-panel" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#1e232d"/><stop offset="1" stop-color="#0d1015"/>
  </linearGradient>
  <radialGradient id="${p}-glow" cx="0.5" cy="0.5" r="0.5">
    <stop offset="0" stop-color="#f4b728" stop-opacity="0.28"/><stop offset="1" stop-color="#f4b728" stop-opacity="0"/>
  </radialGradient>`;

/** A deterministic QR-like pattern (not a real code: decoration only). */
function fakeQr(x: number, y: number, size: number, cells = 13): SVGGElement {
  const g = document.createElementNS(NS, 'g');
  g.setAttribute('class', 'sc-qr');
  const c = size / cells;
  let s = 0x2f6b;
  const rnd = (): number => {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    return s / 0x7fffffff;
  };
  const finder = (fx: number, fy: number): boolean => fx < 4 && fy < 4;
  for (let i = 0; i < cells; i++) {
    for (let j = 0; j < cells; j++) {
      const inF =
        finder(i, j) || finder(cells - 1 - i, j) || finder(i, cells - 1 - j);
      if (inF) continue;
      if (rnd() < 0.48) {
        const r = document.createElementNS(NS, 'rect');
        r.setAttribute('x', String(x + i * c));
        r.setAttribute('y', String(y + j * c));
        r.setAttribute('width', String(c * 0.9));
        r.setAttribute('height', String(c * 0.9));
        g.append(r);
      }
    }
  }
  for (const [fx, fy] of <[number, number][]>[
    [0, 0],
    [cells - 4, 0],
    [0, cells - 4],
  ]) {
    const o = document.createElementNS(NS, 'rect');
    o.setAttribute('x', String(x + fx * c + c * 0.25));
    o.setAttribute('y', String(y + fy * c + c * 0.25));
    o.setAttribute('width', String(c * 3.4));
    o.setAttribute('height', String(c * 3.4));
    o.setAttribute('class', 'sc-qr-f');
    const i = document.createElementNS(NS, 'rect');
    i.setAttribute('x', String(x + fx * c + c * 1.15));
    i.setAttribute('y', String(y + fy * c + c * 1.15));
    i.setAttribute('width', String(c * 1.6));
    i.setAttribute('height', String(c * 1.6));
    g.append(o, i);
  }
  return g;
}

function finish(s: SVGSVGElement): SVGSVGElement {
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('focusable', 'false');
  if (reducedMotion()) s.querySelectorAll('animate, animateMotion, animateTransform').forEach((a) => a.remove());
  return s;
}

const WORDS = ['amber', 'orbit', 'lunar', 'quiet', 'harbor', 'velvet', 'ember', 'meadow', 'cipher', 'dune', 'willow', 'signal'];

/* 1. The heir makes a key and sends a public code. */
export function sceneHeir(): SVGSVGElement {
  const tiles = WORDS.slice(0, 8)
    .map((_, i) => {
      const x = 96 + (i % 4) * 74;
      const y = 34 + Math.floor(i / 4) * 34;
      return `<g class="sc-tile" transform="translate(${x} ${y})"><rect width="64" height="24" rx="6"/><text x="10" y="16">${String(i + 1).padStart(2, '0')}</text><rect class="sc-redact" x="26" y="9" width="30" height="6" rx="3"/></g>`;
    })
    .join('');
  const s = svgFrom(`<svg xmlns="${NS}" viewBox="0 0 480 420" class="scene scene-heir">
  <defs>${GOLD_DEFS('s1')}
    <linearGradient id="s1-shine" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#fff" stop-opacity="0"/><stop offset="0.5" stop-color="#fff" stop-opacity="0.16"/><stop offset="1" stop-color="#fff" stop-opacity="0"/>
    </linearGradient>
    <clipPath id="s1-clip"><rect x="90" y="150" width="300" height="176" rx="16"/></clipPath>
  </defs>
  <ellipse cx="240" cy="238" rx="210" ry="150" fill="url(#s1-glow)"/>
  <g class="sc-words">${tiles}</g>
  <g class="sc-lock" transform="translate(52 52)"><rect x="-13" y="-2" width="26" height="22" rx="5"/><path d="M-8 -2v-7a8 8 0 0 1 16 0v7" fill="none"/></g>
  <text class="sc-cap" x="96" y="124">24 words · they never leave the heir</text>
  <path class="sc-derive" d="M240 128 V146"/>
  <g class="sc-card">
    <rect x="90" y="150" width="300" height="176" rx="16" fill="url(#s1-panel)" class="sc-card-bg"/>
    <g clip-path="url(#s1-clip)"><rect class="sc-sheen" x="-120" y="150" width="120" height="176" fill="url(#s1-shine)"/></g>
    <text class="sc-label" x="112" y="180">HEIR CODE</text>
    <rect x="112" y="196" width="40" height="30" rx="6" fill="url(#s1-gold)"/>
    <path d="M112 206h14M112 216h14M138 196v30" class="sc-chip"/>
    <text class="sc-code" x="112" y="268">age1q7x9m3k…</text>
    <text class="sc-code dim" x="112" y="290">…v9f2k8wl0ch</text>
    <text class="sc-fine" x="112" y="312">public · safe to send</text>
  </g>
  <path class="sc-send" d="M390 262 C 440 262 440 360 396 372"/>
  <g transform="translate(372 378)" class="sc-owner"><circle r="22"/><circle cy="-6" r="7" class="sc-head"/><path d="M-12 12a12 10 0 0 1 24 0" class="sc-head"/></g>
  <text class="sc-cap" x="250" y="410">sent to the owner</text>
</svg>`);
  s.querySelector('.sc-card')?.append(fakeQr(300, 168, 70));
  return finish(s);
}

/* 2. Funds go into a fresh vault; its seed is sealed to time and to the heir. */
export function sceneSeal(): SVGSVGElement {
  const coins = [0, 1, 2]
    .map(
      (i) =>
        `<g class="sc-coin c${i}"><circle cx="240" cy="0" r="13" fill="url(#s2-gold)"/><text x="240" y="5" class="sc-z">Z</text></g>`,
    )
    .join('');
  return finish(
    svgFrom(`<svg xmlns="${NS}" viewBox="0 0 480 420" class="scene scene-seal">
  <defs>${GOLD_DEFS('s2')}</defs>
  <ellipse cx="240" cy="270" rx="200" ry="140" fill="url(#s2-glow)"/>
  <g class="sc-coins">${coins}</g>
  <path class="sc-chain" d="M128 150 C 128 196 168 196 176 206"/>
  <path class="sc-chain" d="M352 150 C 352 196 312 196 304 206"/>
  <g transform="translate(128 106)" class="sc-seal">
    <circle r="44" class="sc-seal-ring"/>
    <circle r="34" class="sc-face"/>
    <g class="sc-ticks"><path d="M0 -30v6M0 30v-6M-30 0h6M30 0h-6"/></g>
    <path class="sc-hand" d="M0 0V-22"/>
    <path class="sc-hand short" d="M0 0H14"/>
    <circle r="3" fill="#f4b728"/>
  </g>
  <text class="sc-cap mid" x="128" y="40">time lock</text>
  <text class="sc-fine mid" x="128" y="56">drand round</text>
  <g transform="translate(352 106)" class="sc-seal">
    <circle r="44" class="sc-seal-ring"/>
    <circle r="34" class="sc-face"/>
    <circle cy="-8" r="9" class="sc-keyhole"/>
    <path d="M-6 -2 L-9 18 H9 L6 -2Z" class="sc-keyhole"/>
  </g>
  <text class="sc-cap mid" x="352" y="40">heir lock</text>
  <text class="sc-fine mid" x="352" y="56">age · heir code</text>
  <g class="sc-vault">
    <rect x="150" y="196" width="180" height="168" rx="20" fill="url(#s2-panel)" class="sc-vault-body"/>
    <rect x="226" y="184" width="28" height="8" rx="4" class="sc-slot"/>
    <circle cx="240" cy="280" r="50" class="sc-door"/>
    <circle cx="240" cy="280" r="36" class="sc-door inner"/>
    <g class="sc-wheel"><path d="M240 250v60M210 280h60M219 259l42 42M261 259l-42 42"/><circle cx="240" cy="280" r="8" fill="url(#s2-gold)"/></g>
    <rect x="166" y="352" width="16" height="16" rx="3" class="sc-foot"/>
    <rect x="298" y="352" width="16" height="16" rx="3" class="sc-foot"/>
  </g>
  <text class="sc-cap mid" x="240" y="404">a fresh vault, its seed sealed twice</text>
</svg>`),
  );
}

/* 3. Flip: funds roll to a new vault; old kits open an empty one. */
export function sceneFlip(): SVGSVGElement {
  const vault = (x: number, cls: string, id: string): string => `
  <g class="sc-vault ${cls}" transform="translate(${x} 0)">
    <rect x="0" y="220" width="130" height="120" rx="16" fill="url(#${id})" class="sc-vault-body"/>
    <circle cx="65" cy="280" r="34" class="sc-door"/>
    <g class="sc-wheel sm"><path d="M65 258v44M43 280h44"/><circle cx="65" cy="280" r="6" class="sc-hub"/></g>
  </g>`;
  const path = 'M110 226 Q240 70 370 226';
  const coins = [0, 0.45, 0.9]
    .map(
      (d) =>
        `<g class="sc-coin-fly"><circle r="11" fill="url(#s3-gold)"/><text y="4" class="sc-z">Z</text>
        <animateMotion dur="2.6s" begin="${d}s" repeatCount="indefinite" path="${path}" keyPoints="0;1" keyTimes="0;1" calcMode="spline" keySplines="0.4 0 0.2 1"/></g>`,
    )
    .join('');
  return finish(
    svgFrom(`<svg xmlns="${NS}" viewBox="0 0 480 420" class="scene scene-flip">
  <defs>${GOLD_DEFS('s3')}</defs>
  <ellipse cx="370" cy="290" rx="150" ry="120" fill="url(#s3-glow)"/>
  <path d="${path}" class="sc-arc"/>
  ${vault(45, 'old', 's3-panel')}
  ${vault(305, 'new', 's3-panel')}
  ${coins}
  <g class="sc-mini-glass" transform="translate(240 120)">
    <g class="sc-flipper">
      <path d="M-18 -30h36M-18 30h36M-14 -30c0 14 10 20 14 30-4 10-14 16-14 30M14 -30c0 14-10 20-14 30 4 10 14 16 14 30" class="sc-mg"/>
      <path d="M-9 -22h18L0 -4Z" fill="url(#s3-gold)"/>
      <path d="M-11 27h22L0 14Z" fill="url(#s3-gold)"/>
    </g>
  </g>
  <text class="sc-cap mid" x="110" y="364">old vault</text>
  <text class="sc-fine mid" x="110" y="379">empty</text>
  <text class="sc-cap mid gold" x="370" y="364">new vault</text>
  <text class="sc-fine mid" x="370" y="379">new kit, new timer</text>
  <g class="sc-oldkit" transform="translate(128 396)">
    <rect width="34" height="20" rx="3" class="sc-env"/><path d="M0 2 L17 11 L34 2" class="sc-env-flap"/>
    <path d="M-3 23 L37 -3" class="sc-strike"/>
  </g>
  <text class="sc-fine" x="172" y="410">an old kit opens an empty vault</text>
</svg>`),
  );
}

/* A sealed kit: an envelope with an intact wax seal and two lock rings. */
export function sceneKit(): SVGSVGElement {
  return finish(
    svgFrom(`<svg xmlns="${NS}" viewBox="0 0 480 340" class="scene scene-kit">
  <defs>${GOLD_DEFS('sk')}</defs>
  <ellipse cx="240" cy="180" rx="220" ry="150" fill="url(#sk-glow)" class="sk-glow"/>
  <circle cx="240" cy="180" r="118" class="sk-ring a"/>
  <circle cx="240" cy="180" r="138" class="sk-ring b"/>
  <g class="sc-kit">
    <rect x="100" y="100" width="280" height="160" rx="14" fill="url(#sk-panel)" class="sc-env"/>
    <path d="M100 108 L240 196 L380 108" class="sc-env-flap"/>
    <circle cx="240" cy="196" r="38" fill="url(#sk-gold)" class="sk-wax"/>
    <circle cx="240" cy="196" r="28" class="sk-wax-ring"/>
    <path d="M226 186h28l-28 20h28" class="sk-z"/>
  </g>
  <text class="sc-fine mid" x="240" y="300">sealed to a drand round and to an heir code</text>
</svg>`),
  );
}

/* 4. The sand runs out: the heir opens the kit, the seed comes out. */
export function sceneOpen(): SVGSVGElement {
  const tiles = WORDS.map((w, i) => {
    const x = 70 + (i % 4) * 88;
    const y = 30 + Math.floor(i / 4) * 34;
    return `<g transform="translate(${x} ${y})"><g class="sc-tile up t${i}"><rect width="78" height="24" rx="6"/><text x="8" y="16">${String(i + 1).padStart(2, '0')}</text><text x="28" y="16" class="w">${w}</text></g></g>`;
  }).join('');
  return finish(
    svgFrom(`<svg xmlns="${NS}" viewBox="0 0 480 420" class="scene scene-open">
  <defs>${GOLD_DEFS('s4')}</defs>
  <ellipse cx="240" cy="200" rx="230" ry="160" fill="url(#s4-glow)"/>
  <g class="sc-words">${tiles}</g>
  <g class="sc-kit">
    <rect x="110" y="200" width="260" height="150" rx="14" fill="url(#s4-panel)" class="sc-env"/>
    <path d="M110 208 L240 292 L370 208" class="sc-env-flap"/>
    <g class="sc-wax l"><path d="M240 238 a36 36 0 0 0 0 72 l6 -12 -8 -10 7 -14 -7 -12 8 -12z" fill="url(#s4-gold)"/></g>
    <g class="sc-wax r"><path d="M240 238 a36 36 0 0 1 0 72 l6 -12 -8 -10 7 -14 -7 -12 8 -12z" fill="url(#s4-gold)"/></g>
  </g>
  <text class="sc-cap mid" x="240" y="384">the heir opens the kit and restores the seed</text>
  <text class="sc-fine mid" x="240" y="402">in any Zcash wallet</text>
</svg>`),
  );
}
