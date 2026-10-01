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

/**
 * Parse a CONSTANT SVG illustration written in this codebase (never user or
 * network data). No inline style attributes: the CSP blocks them.
 */
export function svgFrom(markup: string): SVGSVGElement {
  const doc = new DOMParser().parseFromString(markup, 'image/svg+xml');
  return document.importNode(doc.documentElement, true) as unknown as SVGSVGElement;
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
