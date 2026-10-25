/**
 * Paper kits and the heir's calendar file. A kit is public JSON around one
 * sealed ciphertext, so it can be printed as a single QR code and read back
 * in the browser. Nothing here touches the network.
 */
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

/** Does `text` fit one QR code at the lowest error correction (version 40)? */
export function fitsQr(text: string): boolean {
  try {
    const qr = qrcode(0, 'L');
    qr.addData(text);
    qr.make();
    return true;
  } catch {
    return false;
  }
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

/** Decode a QR code from an image file: BarcodeDetector if present, else jsQR. */
export async function decodeQrFile(file: File): Promise<string | null> {
  const bmp = await createImageBitmap(file);
  try {
    const det = nativeDetector();
    if (det) {
      try {
        const found = await det.detect(bmp);
        if (found[0]?.rawValue) return found[0].rawValue;
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
    if (!ctx) return null;
    ctx.fillStyle = '#fff';
    ctx.fillRect(0, 0, w, hgt);
    ctx.drawImage(bmp, 0, 0, w, hgt);
    const px = ctx.getImageData(0, 0, w, hgt);
    return await decodeQrPixels(px.data, w, hgt);
  } finally {
    bmp.close();
  }
}

/**
 * Scan with the camera until a QR code is read or `signal` aborts. Returns
 * null when aborted. Throws when the camera cannot be used.
 */
export async function scanCamera(video: HTMLVideoElement, signal: AbortSignal): Promise<string | null> {
  const det = nativeDetector();
  if (!det) throw new Error('no BarcodeDetector');
  const stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' }, audio: false });
  try {
    video.srcObject = stream;
    await video.play();
    while (!signal.aborted) {
      try {
        const found = await det.detect(video);
        if (found[0]?.rawValue) return found[0].rawValue;
      } catch {
        // frame not ready yet
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    return null;
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
