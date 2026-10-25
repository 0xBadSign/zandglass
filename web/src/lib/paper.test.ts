import { describe, expect, it } from 'vitest';
import { makeJsonKit, qrPixels, qrPng, read } from '../test-helpers';
import { decodeQrPixels, fitsQr, kitQrText, unlockIcs } from './paper';
import { parseKit } from './kit';

describe('paper kit QR', () => {
  it('a real JSON kit fits one QR code and shrinks when compacted', async () => {
    const kit = await makeJsonKit();
    const text = kitQrText(kit);
    expect(text.length).toBeLessThanOrEqual(kit.length);
    expect(text.length).toBeLessThan(2900);
    expect(fitsQr(text)).toBe(true);
    expect(/^[\x00-\x7f]*$/.test(text)).toBe(true);
  });

  it('a bare armoured vector fits too', () => {
    expect(fitsQr(kitQrText(read('kit.txt')))).toBe(true);
  });

  it('too much data does not fit', () => {
    expect(fitsQr('x'.repeat(3200))).toBe(false);
    expect(fitsQr('x'.repeat(2900))).toBe(true);
  });

  it('non-ASCII text is escaped to ASCII and parses back', () => {
    const out = kitQrText('{"message":"café ❤"}');
    expect(out).toBe('{"message":"caf\\u00e9 \\u2764"}');
    expect(JSON.parse(out).message).toBe('café ❤');
  });

  it('round trips through the decoder into parseKit', async () => {
    const kit = await makeJsonKit();
    const text = kitQrText(kit);
    const { data, size } = qrPixels(text);
    const decoded = await decodeQrPixels(data, size, size);
    expect(decoded).toBe(text);
    const a = parseKit(decoded as string);
    const b = parseKit(kit);
    expect(a.sealed).toBe(b.sealed);
    expect(a.round).toBe(b.round);
    expect(a.meta?.epoch).toBe(b.meta?.epoch);
  });

  it('qrPng makes a valid PNG', async () => {
    const kit = kitQrText(await makeJsonKit());
    const png = qrPng(kit);
    expect(png.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a');
    expect(png.length).toBeGreaterThan(1000);
  });

  it('returns null when there is no QR code', async () => {
    const data = new Uint8ClampedArray(100 * 100 * 4).fill(255);
    expect(await decodeQrPixels(data, 100, 100)).toBeNull();
  });
});

describe('unlock calendar file', () => {
  const ics = unlockIcs(Date.UTC(2026, 9, 1, 12, 0, 0) / 1000, Date.UTC(2026, 8, 1) / 1000, 'abc@zandglass');

  it('has the unlock time, a neutral title and one alarm', () => {
    expect(ics).toContain('BEGIN:VCALENDAR\r\n');
    expect(ics).toContain('DTSTART:20261001T120000Z\r\n');
    expect(ics).toContain('DTSTAMP:20260901T000000Z\r\n');
    expect(ics).toContain('SUMMARY:Zandglass kit unlocks\r\n');
    expect(ics.match(/BEGIN:VALARM/g)).toHaveLength(1);
    expect(ics.endsWith('END:VCALENDAR\r\n')).toBe(true);
  });

  it('has a random UID and nothing that identifies a plan', () => {
    const a = unlockIcs(1, 1);
    const b = unlockIcs(1, 1);
    expect(a.match(/UID:([0-9a-f]{32})@zandglass/)).not.toBeNull();
    expect(a).not.toEqual(b);
    expect(ics.toLowerCase()).not.toMatch(/plan|[0-9a-f]{32}/);
  });

  it('folds long lines at 75 characters', () => {
    for (const l of ics.split('\r\n')) expect(l.length).toBeLessThanOrEqual(75);
  });
});
