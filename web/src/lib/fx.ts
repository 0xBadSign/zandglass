/**
 * Visual effects: a canvas zandglass with falling grains, an ambient dust
 * field, and a drand round clock. Everything respects prefers-reduced-motion,
 * pauses when off screen or in a hidden tab, and makes no network requests.
 */

export const reducedMotion = (): boolean =>
  window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

/* -------------------------------------------------------------- geometry */

const TOP = 0.07; // glass starts (fraction of height)
const BOT = 0.93; // glass ends
const MID = 0.5;
const NECK = 0.022; // half width of the neck (fraction of width)
const BULB = 0.4; // half width of a bulb

/** Half width of the glass at height y (0..1), as a fraction of the width. */
function halfWidth(y: number): number {
  const s = Math.min(1, Math.abs(y - MID) / (MID - TOP));
  // a round bulb: grows from the neck, then curls back in under the cap
  const bulge = Math.pow(Math.sin((s * Math.PI) / 2), 1.5);
  const shoulder = s > 0.78 ? 1 - 0.28 * Math.pow((s - 0.78) / 0.22, 2) : 1;
  return NECK + (BULB - NECK) * bulge * shoulder;
}

const STEPS = 240;
/** areaFromNeck[i]: area of one bulb between the neck and distance i/STEPS. */
const areaFromNeck: number[] = (() => {
  const out = [0];
  const span = MID - TOP;
  let acc = 0;
  for (let i = 1; i <= STEPS; i++) {
    const y = MID - (span * (i - 0.5)) / STEPS;
    acc += 2 * halfWidth(y) * (span / STEPS);
    out.push(acc);
  }
  return out;
})();
const BULB_AREA = (areaFromNeck[STEPS] ?? 0) * 0.86; // a bulb is never filled to the brim

/** Distance from the neck (0..span) that holds `area` of sand. */
function distanceFor(area: number): number {
  let lo = 0;
  let hi = STEPS;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if ((areaFromNeck[m] ?? 0) < area) lo = m + 1;
    else hi = m;
  }
  return (lo / STEPS) * (MID - TOP);
}

interface Grain {
  x: number;
  y: number;
  vy: number;
  r: number;
  a: number;
}

export interface GlassOptions {
  /** Loop: drain, then flip and start again. Otherwise progress is set from outside. */
  autoplay?: boolean;
  /** Seconds for one full drain in autoplay mode. */
  period?: number;
  /** Extra class names on the wrapper. */
  className?: string;
}

/**
 * A zandglass drawn on canvas. `progress` is the fraction of sand that has
 * fallen (0..1). `flip()` turns it over with a CSS rotation.
 */
export class GlassCanvas {
  readonly el: HTMLDivElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private progress = 0.18;
  private shown = 0.18;
  private grains: Grain[] = [];
  private raf = 0;
  private last = 0;
  private visible = true;
  private flowing = true;
  private flipping = false;
  private readonly still = reducedMotion();
  private w = 0;
  private h = 0;
  private dpr = 1;

  constructor(private readonly opts: GlassOptions = {}) {
    this.el = document.createElement('div');
    this.el.className = `sgc ${opts.className ?? ''}`.trim();
    this.el.setAttribute('aria-hidden', 'true');
    this.canvas = document.createElement('canvas');
    this.el.append(this.canvas);
    const ctx = this.canvas.getContext('2d');
    if (!ctx) throw new Error('no canvas');
    this.ctx = ctx;
    new ResizeObserver(() => this.resize()).observe(this.el);
    new IntersectionObserver((e) => {
      this.visible = e[0]?.isIntersecting ?? true;
      if (this.visible) this.kick();
    }).observe(this.el);
    document.addEventListener('visibilitychange', () => this.kick());
    if (opts.autoplay) this.progress = this.shown = 0.08;
  }

  /** Fraction of sand fallen, 0..1. */
  set(progress: number): void {
    this.progress = Math.max(0, Math.min(1, progress));
    if (this.still) this.shown = this.progress;
    this.kick();
  }

  /** Grains fall only while flowing (a stopped or empty glass is quiet). */
  setFlowing(on: boolean): void {
    this.flowing = on;
    this.kick();
  }

  /** Turn the glass over: the fallen sand becomes the top again. */
  flip(): Promise<void> {
    if (this.flipping) return Promise.resolve();
    if (this.still) {
      this.progress = this.shown = 1 - this.progress;
      this.draw();
      return Promise.resolve();
    }
    this.flipping = true;
    this.grains = [];
    this.el.classList.add('sgc-flip');
    return new Promise((done) => {
      window.setTimeout(() => {
        this.el.classList.add('sgc-noanim');
        this.el.classList.remove('sgc-flip');
        // autoplay: the fallen sand is now on top; driven: jump to the new target
        if (this.opts.autoplay) this.progress = this.shown = 1 - this.shown;
        else this.shown = this.progress;
        this.draw();
        void this.el.offsetWidth;
        this.el.classList.remove('sgc-noanim');
        this.flipping = false;
        this.kick();
        done();
      }, 1100);
    });
  }

  private resize(): void {
    const r = this.el.getBoundingClientRect();
    this.dpr = Math.min(2, window.devicePixelRatio || 1);
    this.w = Math.max(1, r.width);
    this.h = Math.max(1, r.height);
    this.canvas.width = Math.round(this.w * this.dpr);
    this.canvas.height = Math.round(this.h * this.dpr);
    this.draw();
  }

  private kick(): void {
    if (this.raf || this.still || !this.visible || document.hidden) {
      if (this.still) this.draw();
      return;
    }
    this.last = performance.now();
    this.raf = requestAnimationFrame((t) => this.frame(t));
  }

  private frame(t: number): void {
    this.raf = 0;
    const dt = Math.min(0.05, (t - this.last) / 1000);
    this.last = t;
    if (this.opts.autoplay && !this.flipping) {
      this.progress = Math.min(1, this.progress + dt / (this.opts.period ?? 16));
      if (this.progress >= 1 && this.shown >= 0.999) void this.flip();
    }
    // ease the drawn level toward the target
    this.shown += (this.progress - this.shown) * Math.min(1, dt * 3);
    if (Math.abs(this.progress - this.shown) < 0.0005) this.shown = this.progress;
    this.step(dt);
    this.draw();
    if (this.visible && !document.hidden) this.raf = requestAnimationFrame((n) => this.frame(n));
  }

  /* -------------------------------------------------------------- physics */

  private bottomSurface(x: number): number {
    // Height of the mound (fraction of height) at horizontal offset x (fraction of width from the center).
    const area = this.shown * BULB_AREA;
    const d = distanceFor(area);
    const base = BOT - d; // flat level
    const peak = Math.min(0.06, 0.02 + this.shown * 0.05);
    const hw = halfWidth(base);
    const k = Math.max(0, 1 - Math.pow(Math.abs(x) / Math.max(hw, 0.05), 1.6));
    return base - peak * k + peak * 0.35;
  }

  private step(dt: number): void {
    const running = this.flowing && !this.flipping && this.shown < 0.995 && this.shown > 0.0005;
    if (running) {
      const n = Math.random() < 0.5 ? 2 : 3;
      for (let i = 0; i < n; i++) {
        this.grains.push({
          x: (Math.random() - 0.5) * NECK * 0.8,
          y: MID + 0.004,
          vy: 0.1 + Math.random() * 0.08,
          r: 0.6 + Math.random() * 0.9,
          a: 0.55 + Math.random() * 0.45,
        });
      }
    }
    const g = 1.9;
    this.grains = this.grains.filter((p) => {
      p.vy += g * dt;
      p.y += p.vy * dt;
      p.x += (Math.random() - 0.5) * 0.0015;
      return p.y < this.bottomSurface(p.x);
    });
  }

  /* --------------------------------------------------------------- render */

  private draw(): void {
    const { ctx, w, h, dpr } = this;
    if (!w || !h) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const S = Math.min(w, h * 0.66); // glass width in px
    const ox = (w - S) / 2;
    const X = (fx: number): number => ox + S * (0.5 + fx);
    const Y = (fy: number): number => h * fy;

    // glow
    const glow = ctx.createRadialGradient(w / 2, h * 0.55, 0, w / 2, h * 0.55, S * 0.75);
    glow.addColorStop(0, 'rgba(244,183,40,0.22)');
    glow.addColorStop(1, 'rgba(244,183,40,0)');
    ctx.fillStyle = glow;
    ctx.fillRect(0, 0, w, h);

    const outline = (side: 1 | -1, from: number, to: number, cont = false): void => {
      const n = 80;
      for (let i = 0; i <= n; i++) {
        const y = from + ((to - from) * i) / n;
        const x = X(side * halfWidth(y));
        if (i === 0 && !cont) ctx.moveTo(x, Y(y));
        else ctx.lineTo(x, Y(y));
      }
    };
    const glassPath = (): void => {
      ctx.beginPath();
      outline(-1, TOP, BOT);
      outline(1, BOT, TOP, true);
      ctx.closePath();
    };

    // glass body tint
    glassPath();
    const tint = ctx.createLinearGradient(X(-BULB), 0, X(BULB), 0);
    tint.addColorStop(0, 'rgba(255,255,255,0.07)');
    tint.addColorStop(0.35, 'rgba(255,255,255,0.015)');
    tint.addColorStop(0.7, 'rgba(255,255,255,0.03)');
    tint.addColorStop(1, 'rgba(255,255,255,0.08)');
    ctx.fillStyle = tint;
    ctx.fill();

    // sand, clipped to the glass
    ctx.save();
    glassPath();
    ctx.clip();
    const sand = ctx.createLinearGradient(0, Y(TOP), 0, Y(BOT));
    sand.addColorStop(0, '#ffe08a');
    sand.addColorStop(0.5, '#f4b728');
    sand.addColorStop(1, '#c98912');
    ctx.fillStyle = sand;

    // top: what is left, with a dip in the middle where it drains
    const left = (1 - this.shown) * BULB_AREA;
    if (left > 0.0002) {
      const d = distanceFor(left);
      const lvl = MID - d;
      const dip = Math.min(0.035, d * 0.25);
      const hw = halfWidth(lvl);
      ctx.beginPath();
      ctx.moveTo(X(-hw - 0.02), Y(lvl));
      ctx.quadraticCurveTo(X(0), Y(lvl + dip * 2), X(hw + 0.02), Y(lvl));
      ctx.lineTo(X(hw + 0.02), Y(MID + 0.01));
      ctx.lineTo(X(-hw - 0.02), Y(MID + 0.01));
      ctx.closePath();
      ctx.fill();
    }
    // bottom mound
    if (this.shown > 0.0005) {
      ctx.beginPath();
      const n = 60;
      for (let i = 0; i <= n; i++) {
        const fx = -BULB + (2 * BULB * i) / n;
        const y = this.bottomSurface(fx);
        if (i === 0) ctx.moveTo(X(fx), Y(y));
        else ctx.lineTo(X(fx), Y(y));
      }
      ctx.lineTo(X(BULB), Y(BOT + 0.01));
      ctx.lineTo(X(-BULB), Y(BOT + 0.01));
      ctx.closePath();
      ctx.fill();
    }
    // stream through the neck
    const running = this.flowing && !this.flipping && this.shown < 0.995 && this.shown > 0.0005;
    if (running) {
      const end = this.bottomSurface(0);
      const st = ctx.createLinearGradient(0, Y(MID), 0, Y(end));
      st.addColorStop(0, 'rgba(255,214,110,0.95)');
      st.addColorStop(1, 'rgba(244,183,40,0.25)');
      ctx.strokeStyle = st;
      ctx.lineWidth = Math.max(1, S * 0.006);
      ctx.beginPath();
      ctx.moveTo(X(0), Y(MID - 0.01));
      ctx.lineTo(X(0), Y(end));
      ctx.stroke();
    }
    for (const p of this.grains) {
      ctx.globalAlpha = p.a;
      ctx.fillStyle = '#ffd875';
      ctx.beginPath();
      ctx.arc(X(p.x), Y(p.y), p.r * (S / 320), 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.globalAlpha = 1;
    ctx.restore();

    // glass edges
    ctx.lineWidth = Math.max(1.2, S * 0.008);
    ctx.lineCap = 'round';
    const edge = ctx.createLinearGradient(0, Y(TOP), 0, Y(BOT));
    edge.addColorStop(0, 'rgba(255,240,210,0.55)');
    edge.addColorStop(0.5, 'rgba(255,240,210,0.25)');
    edge.addColorStop(1, 'rgba(255,240,210,0.5)');
    ctx.strokeStyle = edge;
    for (const side of [-1, 1] as const) {
      ctx.beginPath();
      outline(side, TOP, BOT);
      ctx.stroke();
    }
    // highlight
    ctx.strokeStyle = 'rgba(255,255,255,0.22)';
    ctx.lineWidth = Math.max(1.5, S * 0.014);
    ctx.beginPath();
    for (let i = 0; i <= 30; i++) {
      const y = TOP + 0.05 + (0.3 * i) / 30;
      const x = X(-halfWidth(y) * 0.78);
      if (i === 0) ctx.moveTo(x, Y(y));
      else ctx.lineTo(x, Y(y));
    }
    ctx.stroke();

    // caps and posts
    const capW = BULB * 2 + 0.12;
    const capH = 0.035;
    const cap = (y: number): void => {
      const g2 = ctx.createLinearGradient(0, Y(y), 0, Y(y + capH));
      g2.addColorStop(0, '#3a3226');
      g2.addColorStop(0.5, '#1c1813');
      g2.addColorStop(1, '#0e0c09');
      ctx.fillStyle = g2;
      roundRect(ctx, X(-capW / 2), Y(y), S * capW, h * capH, h * 0.01);
      ctx.fill();
      ctx.strokeStyle = 'rgba(244,183,40,0.55)';
      ctx.lineWidth = 1;
      ctx.stroke();
    };
    ctx.strokeStyle = 'rgba(244,183,40,0.35)';
    ctx.lineWidth = Math.max(1.5, S * 0.012);
    for (const fx of [-capW / 2 + 0.03, capW / 2 - 0.03]) {
      ctx.beginPath();
      ctx.moveTo(X(fx), Y(TOP - 0.01));
      ctx.lineTo(X(fx), Y(BOT + 0.01));
      ctx.stroke();
    }
    cap(TOP - capH);
    cap(BOT);
  }
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

/* ---------------------------------------------------------- ambient dust */

/** A fixed, full-page canvas of slow gold dust behind the content. */
export function mountDust(): void {
  if (reducedMotion()) return;
  const c = document.createElement('canvas');
  c.className = 'dust';
  c.setAttribute('aria-hidden', 'true');
  document.body.prepend(c);
  const ctx = c.getContext('2d');
  if (!ctx) return;
  let w = 0;
  let h = 0;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const resize = (): void => {
    w = window.innerWidth;
    h = window.innerHeight;
    c.width = w * dpr;
    c.height = h * dpr;
  };
  resize();
  window.addEventListener('resize', resize);
  const N = Math.round(Math.min(90, (w * h) / 16000));
  const ps = Array.from({ length: N }, () => ({
    x: Math.random(),
    y: Math.random(),
    z: 0.3 + Math.random() * 0.7,
    s: Math.random() * Math.PI * 2,
  }));
  let mx = 0;
  let my = 0;
  window.addEventListener('pointermove', (e) => {
    mx = e.clientX / w - 0.5;
    my = e.clientY / h - 0.5;
  });
  let last = performance.now();
  const frame = (t: number): void => {
    const dt = Math.min(0.05, (t - last) / 1000);
    last = t;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const sy = window.scrollY;
    for (const p of ps) {
      p.y -= dt * 0.012 * p.z;
      p.s += dt * 0.6;
      if (p.y < -0.05) {
        p.y = 1.05;
        p.x = Math.random();
      }
      const px = (p.x + Math.sin(p.s) * 0.004 + mx * 0.02 * p.z) * w;
      const py = ((p.y + my * 0.02 * p.z) * h - sy * 0.15 * p.z) % (h + 40);
      const yy = py < -20 ? py + h + 40 : py;
      const a = 0.12 + 0.4 * p.z * (0.6 + 0.4 * Math.sin(p.s * 1.7));
      ctx.fillStyle = `rgba(244,190,70,${a.toFixed(3)})`;
      ctx.beginPath();
      ctx.arc(px, yy, 0.6 + p.z * 1.3, 0, Math.PI * 2);
      ctx.fill();
    }
    if (!document.hidden) requestAnimationFrame(frame);
  };
  document.addEventListener('visibilitychange', () => {
    if (!document.hidden) {
      last = performance.now();
      requestAnimationFrame(frame);
    }
  });
  requestAnimationFrame(frame);
}

/* ---------------------------------------------------------- drand clock */

const GENESIS = 1692803367;
const PERIOD = 3;

/** The current drand quicknet round, computed locally (no request). */
export const drandRoundNow = (ms = Date.now()): number => Math.floor((ms / 1000 - GENESIS) / PERIOD) + 1;

/** A small live chip: "drand round 13,512,004" with a tick every 3 s. */
export function drandTicker(): HTMLElement {
  const el = document.createElement('div');
  el.className = 'drand-tick';
  const dot = document.createElement('span');
  dot.className = 'dt-dot';
  const label = document.createElement('span');
  label.className = 'dt-label';
  label.textContent = 'drand round';
  const num = document.createElement('span');
  num.className = 'dt-num';
  const bar = document.createElement('span');
  bar.className = 'dt-bar';
  const fill = document.createElement('span');
  bar.append(fill);
  el.append(dot, label, num, bar);
  let shown = 0;
  const paint = (): void => {
    const now = Date.now();
    const r = drandRoundNow(now);
    if (r !== shown) {
      shown = r;
      num.textContent = r.toLocaleString('en-US');
      el.classList.remove('tick');
      void el.offsetWidth;
      el.classList.add('tick');
    }
    const into = ((now / 1000 - GENESIS) % PERIOD) / PERIOD;
    fill.style.transform = `scaleX(${into.toFixed(3)})`;
  };
  paint();
  window.setInterval(paint, 100);
  return el;
}

/* ------------------------------------------------------------ reveal */

/** Add `.in` to elements with `.reveal` as they scroll into view. */
export function revealOnScroll(root: ParentNode = document): void {
  const els = root.querySelectorAll<HTMLElement>('.reveal');
  if (reducedMotion() || !('IntersectionObserver' in window)) {
    els.forEach((e) => e.classList.add('in'));
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add('in');
          io.unobserve(e.target);
        }
      }
    },
    { rootMargin: '0px 0px -10% 0px' },
  );
  els.forEach((e) => io.observe(e));
}
