/**
 * Paper kits and the heir's calendar file. A kit is public JSON around one
 * sealed ciphertext, so it can be printed as a few QR codes and read back
 * in the browser. Nothing here touches the network.
 */
import { sha256 } from '@noble/hashes/sha2.js';
import qrcode from 'qrcode-generator';

/** Compact, ASCII-only JSON of a kit (smaller QR; same parse result). */
export function kitQrText(kitText: string): string {
  const t = kitText.replace(/^﻿/, '').trim();
  let compact = t;
  try {
    compact = JSON.stringify(JSON.parse(t));
  } catch {
    // a bare armour string is encoded as it is
  }
  return compact.replace(/[\u007f-￿]/g, (c) => `\\u${c.charCodeAt(0).toString(16).padStart(4, '0')}`);
}

/** Does `text` fit one QR code at the given error correction (default: lowest, version 40)? */
export function fitsQr(text: string, ecc: 'L' | 'M' = 'L'): boolean {
  try {
    const qr = qrcode(0, ecc);
    qr.addData(text);
    qr.make();
    return true;
  } catch {
    return false;
  }
}

/* ------------------------------------------------------------ parts */

/** Kit bytes per QR code. A real kit is about 4.5 KB, so it takes 5 codes of about version 26. */
export const PART_BYTES = 1000;
/** Error correction of the part codes (M survives a crease or a smudge on paper). */
export const PART_ECC = 'M' as const;
/** More parts than this means the kit is too big for paper. */
export const MAX_PARTS = 8;

const PART_RE = /^ZGK1:(\d{1,2})\/(\d{1,2}):([0-9a-f]{8}):([\s\S]*)$/;

/** First 8 hex characters of the SHA-256 of the full kit text. */
export function kitHash(text: string): string {
  return Array.from(sha256(new TextEncoder().encode(text)).slice(0, 4), (b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * The texts of the QR codes of a kit text (ASCII, as `kitQrText` makes it).
 * A short text is one plain code; otherwise `ZGK1:i/n:hash:chunk` parts.
 */
export function splitKit(text: string): string[] {
  if (text.length <= PART_BYTES) return [text];
  const hash = kitHash(text);
  const n = Math.ceil(text.length / PART_BYTES);
  const size = Math.ceil(text.length / n);
  const out: string[] = [];
  for (let i = 0; i < n; i++) out.push(`ZGK1:${i + 1}/${n}:${hash}:${text.slice(i * size, (i + 1) * size)}`);
  return out;
}

export interface KitPart {
  i: number;
  n: number;
  hash: string;
  chunk: string;
}

/** Parse a `ZGK1:` part; null for anything else (or a malformed header). */
export function parsePart(raw: string): KitPart | null {
  const m = PART_RE.exec(raw);
  if (!m) return null;
  const i = Number(m[1]);
  const n = Number(m[2]);
  if (i < 1 || n < 2 || i > n) return null;
  return { i, n, hash: m[3] as string, chunk: m[4] as string };
}

export type AddResult =
  | { status: 'plain'; text: string }
  | { status: 'progress'; n: number; have: number[]; missing: number[] }
  | { status: 'done'; text: string }
  | { status: 'bad' };

/** Collects scanned codes. Parts of different kits are kept apart by their hash. */
export class PartAssembler {
  private sets = new Map<string, { n: number; chunks: Map<number, string> }>();

  add(raw: string): AddResult {
    if (!raw.startsWith('ZGK1:')) return { status: 'plain', text: raw };
    const part = parsePart(raw);
    if (!part) return { status: 'bad' };
    let set = this.sets.get(part.hash);
    if (!set) {
      set = { n: part.n, chunks: new Map() };
      this.sets.set(part.hash, set);
    }
    if (set.n !== part.n) return { status: 'bad' };
    set.chunks.set(part.i, part.chunk);
    if (set.chunks.size < set.n) {
      const have = [...set.chunks.keys()].sort((a, b) => a - b);
      const missing = Array.from({ length: set.n }, (_, k) => k + 1).filter((k) => !set.chunks.has(k));
      return { status: 'progress', n: set.n, have, missing };
    }
    const text = Array.from({ length: set.n }, (_, k) => set.chunks.get(k + 1) as string).join('');
    this.sets.delete(part.hash);
    return kitHash(text) === part.hash ? { status: 'done', text } : { status: 'bad' };
  }
}

/** Assemble a complete list of part texts (any order). Null when anything is wrong. */
export function assembleKit(parts: string[]): string | null {
  const asm = new PartAssembler();
  let last: AddResult = { status: 'bad' };
  for (const p of parts) {
    last = asm.add(p);
    if (last.status === 'bad') return null;
  }
  return last.status === 'done' || last.status === 'plain' ? last.text : null;
}

/* ------------------------------------------------------------ reading */

type JsQr = (data: Uint8ClampedArray, width: number, height: number) => { data: string } | null;

/** Decode the QR code in RGBA pixels with the pure-JS decoder. */
export async function decodeQrPixels(data: Uint8ClampedArray, width: number, height: number): Promise<string | null> {
  const mod = (await import('jsqr')) as unknown as { default: JsQr };
  return mod.default(data, width, height)?.data ?? null;
}

interface Detector {
  detect(src: ImageBitmapSource): Promise<{ rawValue: string }[]>;
}

function nativeDetector(): Detector | null {
  const D = (window as unknown as { BarcodeDetector?: new (o: { formats: string[] }) => Detector }).BarcodeDetector;
  if (!D) return null;
  try {
    return new D({ formats: ['qr_code'] });
  } catch {
    return null;
  }
}

export function hasCameraScan(): boolean {
  return nativeDetector() !== null && typeof navigator.mediaDevices?.getUserMedia === 'function';
}

/** Decode every QR code of an image file: BarcodeDetector finds all of them, jsQR one. */
export async function decodeQrFileAll(file: File): Promise<string[]> {
  const bmp = await createImageBitmap(file);
  try {
    const det = nativeDetector();
    if (det) {
      try {
        const found = (await det.detect(bmp)).map((f) => f.rawValue).filter(Boolean);
        if (found.length) return found;
      } catch {
        // fall through to the JS decoder
      }
    }
    // Large photos are scaled down: the decoder is slower than it is picky.
    const scale = Math.min(1, 1600 / Math.max(bmp.width, bmp.height));
    const w = Math.max(1, Math.round(bmp.width * scale));
    const hgt = Math.max(1, Math.round(bmp.height * scale));
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = hgt;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return [];
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, w, hgt);
    ctx.drawImage(bmp, 0, 0, w, hgt);
    const px = ctx.getImageData(0, 0, w, hgt);
    const one = await decodeQrPixels(px.data, w, hgt);
    return one ? [one] : [];
  } finally {
    bmp.close();
  }
}

/**
 * Scan with the camera, calling `onCode` for each code read, until it
 * returns true or `signal` aborts. Throws when the camera cannot be used.
 */
export async function scanCamera(video: HTMLVideoElement, signal: AbortSignal, onCode: (text: string) => boolean | void): Promise<void> {
  const det = nativeDetector();
  if (!det) throw new Error('no BarcodeDetector');
  const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' }, audio: false });
  try {
    video.srcObject = stream;
    await video.play();
    while (!signal.aborted) {
      try {
        for (const f of await det.detect(video)) {
          if (f.rawValue && onCode(f.rawValue) === true) return;
        }
      } catch {
        // frame not ready yet
      }
      await new Promise((r) => setTimeout(r, 250));
    }
  } finally {
    for (const t of stream.getTracks()) t.stop();
    video.srcObject = null;
  }
}

/* ------------------------------------------------------------ calendar */

const pad = (n: number): string => String(n).padStart(2, '0');

/** 20261001T120000Z */
function icsTime(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}T${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}Z`;
}

function randomUid(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  return `${Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')}@zandglass`;
}

/** Fold a content line at 75 octets (RFC 5545 3.1). Input is ASCII here. */
function fold(text: string): string {
  let out = '';
  let width = 0;
  for (const ch of text) {
    if (width + 1 > 75) {
      out += '\r\n ';
      width = 1;
    }
    out += ch;
    width += 1;
  }
  return `${out}\r\n`;
}

/**
 * An iCalendar file with one event at the unlock time and one alarm. It holds
 * a neutral title, a random UID and no plan id: a cloud calendar must not be
 * able to link it to a kit.
 */
export function unlockIcs(unlockSec: number, nowSec: number, uid: string = randomUid()): string {
  const lines = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Zandglass//Heir//EN',
    'CALSCALE:GREGORIAN',
    'BEGIN:VEVENT',
    `UID:${uid}`,
    `DTSTAMP:${icsTime(nowSec)}`,
    `DTSTART:${icsTime(unlockSec)}`,
    `DTEND:${icsTime(unlockSec + 15 * 60)}`,
    'SUMMARY:Zandglass kit unlocks',
    'DESCRIPTION:The kit can be opened from now on. Open the claim page and enter your heir phrase.',
    'BEGIN:VALARM',
    'ACTION:DISPLAY',
    'DESCRIPTION:Zandglass kit unlocks',
    'TRIGGER:PT0S',
    'END:VALARM',
    'END:VEVENT',
    'END:VCALENDAR',
  ];
  return lines.map(fold).join('');
}
