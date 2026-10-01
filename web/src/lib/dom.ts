/** Tiny DOM helpers. No innerHTML, no inline styles: friendly to a strict CSP. */

export type Child = Node | string | null | false | undefined;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'class') el.className = String(value);
    else if (key.startsWith('on') && typeof value === 'function') {
      el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (value === true) el.setAttribute(key, '');
    else el.setAttribute(key, String(value));
  }
  append(el, ...children);
  return el;
}

export function append(el: Element, ...children: Child[]): void {
  for (const c of children) {
    if (c === null || c === false || c === undefined) continue;
    el.append(typeof c === 'string' ? document.createTextNode(c) : c);
  }
}

export function clear(el: Element): void {
  el.replaceChildren();
}

export function svg(tag: string, attrs: Record<string, string>, ...kids: SVGElement[]): SVGElement {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.append(...kids);
  return el;
}

/** Copy text to the clipboard, with a fallback for restricted contexts. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = h('textarea', { 'aria-hidden': 'true', tabindex: '-1', class: 'offscreen' });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    let ok = false;
    try {
      ok = document.execCommand('copy');
    } catch {
      ok = false;
    }
    ta.remove();
    return ok;
  }
}

/** A button that copies `getText()` and confirms with a short label change. */
export function copyButton(label: string, getText: () => string, cls = 'btn'): HTMLButtonElement {
  const btn = h('button', { type: 'button', class: cls }, label);
  let timer: number | undefined;
  btn.addEventListener('click', () => {
    void copyText(getText()).then((ok) => {
      btn.textContent = ok ? 'Copied' : 'Copy failed: select and copy by hand';
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        btn.textContent = label;
      }, 2200);
    });
  });
  return btn;
}

/**
 * The large animated sandglass for the landing hero: the top bulb drains, a
 * stream of grains falls through the neck and the bottom mound grows. All
 * motion is CSS (see site.css), so prefers-reduced-motion stops it.
 */
export function sandglassArt(): SVGElement {
  const s = svg('svg', {
    viewBox: '0 0 200 280',
    class: 'sg-art',
    'aria-hidden': 'true',
    focusable: 'false',
  });
  const defs = svg(
    'defs',
    {},
    svg(
      'linearGradient',
      { id: 'sg-sand', x1: '0', y1: '0', x2: '0', y2: '1' },
      svg('stop', { offset: '0', 'stop-color': '#ffd873' }),
      svg('stop', { offset: '1', 'stop-color': '#d99a17' }),
    ),
    svg(
      'linearGradient',
      { id: 'sg-glass', x1: '0', y1: '0', x2: '1', y2: '0' },
      svg('stop', { offset: '0', 'stop-color': '#ffffff', 'stop-opacity': '0.55' }),
      svg('stop', { offset: '0.5', 'stop-color': '#ffffff', 'stop-opacity': '0.12' }),
      svg('stop', { offset: '1', 'stop-color': '#ffffff', 'stop-opacity': '0.4' }),
    ),
    svg(
      'radialGradient',
      { id: 'sg-glow', cx: '0.5', cy: '0.5', r: '0.5' },
      svg('stop', { offset: '0', 'stop-color': '#f4b728', 'stop-opacity': '0.32' }),
      svg('stop', { offset: '1', 'stop-color': '#f4b728', 'stop-opacity': '0' }),
    ),
    svg(
      'clipPath',
      { id: 'sg-top' },
      svg('path', { d: 'M52 26 C52 80 94 112 98 138 L102 138 C106 112 148 80 148 26 Z' }),
    ),
    svg(
      'clipPath',
      { id: 'sg-bottom' },
      svg('path', { d: 'M98 142 C94 168 52 200 52 254 L148 254 C148 200 106 168 102 142 Z' }),
    ),
  );
  const glow = svg('ellipse', { cx: '100', cy: '150', rx: '96', ry: '120', fill: 'url(#sg-glow)', class: 'sg-glow' });
  const topSand = svg(
    'g',
    { 'clip-path': 'url(#sg-top)' },
    svg('path', { d: 'M40 64 Q100 52 160 64 L160 150 L40 150 Z', fill: 'url(#sg-sand)', class: 'sg-top' }),
  );
  const bottomSand = svg(
    'g',
    { 'clip-path': 'url(#sg-bottom)' },
    svg('path', { d: 'M40 256 L40 236 Q100 168 160 236 L160 256 Z', fill: 'url(#sg-sand)', class: 'sg-bottom' }),
  );
  const stream = svg('path', { d: 'M100 138 V246', class: 'sg-stream' });
  const grains = svg('g', { class: 'sg-grains' });
  for (let i = 0; i < 9; i++) {
    grains.append(svg('circle', { cx: String(99 + (i % 3)), cy: '140', r: i % 2 ? '1.1' : '1.5' }));
  }
  const glass = svg('path', {
    d: 'M52 26 C52 80 94 112 98 140 C94 168 52 200 52 254 M148 26 C148 80 106 112 102 140 C106 168 148 200 148 254',
    fill: 'none',
    stroke: 'url(#sg-glass)',
    'stroke-width': '2.2',
    'stroke-linecap': 'round',
  });
  const shine = svg('path', {
    d: 'M62 40 C64 74 86 100 94 122',
    fill: 'none',
    class: 'sg-shine',
    'stroke-width': '3',
    'stroke-linecap': 'round',
  });
  const capTop = svg('rect', { x: '34', y: '12', width: '132', height: '14', rx: '7', class: 'sg-cap' });
  const capBottom = svg('rect', { x: '34', y: '254', width: '132', height: '14', rx: '7', class: 'sg-cap' });
  s.append(defs, glow, topSand, bottomSand, stream, grains, glass, shine, capTop, capBottom);
  return s;
}

/** An inline hourglass icon. `level` (0..1) is how much sand has fallen. */
export function hourglass(level = 0.35): SVGElement {
  const top = Math.max(0, 1 - level);
  const topH = 15 * top;
  const botH = 15 * level;
  const s = svg('svg', {
    viewBox: '0 0 48 64',
    width: '48',
    height: '64',
    class: 'hourglass',
    'aria-hidden': 'true',
    focusable: 'false',
  });
  const glass = svg('path', {
    d: 'M10 6h28M10 58h28M13 6c0 12 8 18 11 26-3 8-11 14-11 26M35 6c0 12-8 18-11 26 3 8 11 14 11 26',
    class: 'hg-glass',
    fill: 'none',
    'stroke-width': '2.5',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
  });
  const sandTop = svg('path', {
    d: `M${24 - 10 * top} ${26 - topH}h${20 * top}L24 30z`,
    class: 'hg-sand',
  });
  const stream = svg('path', { d: 'M24 30v20', class: 'hg-stream', 'stroke-width': '1.5' });
  const sandBot = svg('path', {
    d: `M${24 - 11 * Math.min(1, level + 0.1)} 55h${22 * Math.min(1, level + 0.1)}L24 ${55 - botH - 2}z`,
    class: 'hg-sand',
  });
  s.append(sandTop, stream, sandBot, glass);
  return s;
}
