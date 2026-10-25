import qrcode from 'qrcode-generator';
import { describe, expect, it } from 'vitest';
import { makeJsonKit, padSealed, paddedRealKit, qrPixels, qrPng, read } from '../test-helpers';
import { MAX_PARTS, PART_BYTES, PART_ECC, PartAssembler, assembleKit, decodeQrPixels, fitsQr, kitHash, kitQrText, parsePart, splitKit, unlockIcs } from './paper';
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

describe('multi-part paper kit', () => {
  const real = kitQrText(paddedRealKit());

  it('a production-size kit does not fit one QR code', () => {
    expect(real.length).toBeGreaterThan(4300);
    expect(fitsQr(real)).toBe(false);
  });

  it('splits into a few parts that each fit a mid-size QR code', () => {
    const parts = splitKit(real);
    expect(parts.length).toBeGreaterThanOrEqual(3);
    expect(parts.length).toBeLessThanOrEqual(MAX_PARTS);
    const hash = kitHash(real);
    parts.forEach((p, i) => {
      expect(p.startsWith(`ZGK1:${i + 1}/${parts.length}:${hash}:`)).toBe(true);
      expect(p.length).toBeLessThanOrEqual(PART_BYTES + 30);
      expect(/^[\x00-\x7f]*$/.test(p)).toBe(true);
      const qr = qrcode(0, PART_ECC);
      qr.addData(p);
      qr.make();
      expect(qr.getModuleCount()).toBeLessThanOrEqual(25 * 4 + 17 + 4 * 4); // at most version 29
    });
  });

  it('a small text stays one plain code', () => {
    expect(splitKit('{"a":1}')).toEqual(['{"a":1}']);
  });

  it('round trips the real signed kit in order and out of order', () => {
    const signed = kitQrText(read('TEST_ONLY_owner_kit.zandglass'));
    for (const text of [signed, real]) {
      const parts = splitKit(text);
      expect(assembleKit(parts)).toBe(text);
      expect(assembleKit([...parts].reverse())).toBe(text);
      const shuffled = [...parts.slice(1), parts[0] as string];
      expect(assembleKit(shuffled)).toBe(text);
    }
    expect(parseKit(assembleKit(splitKit(real)) as string).sealed).toBe(parseKit(paddedRealKit()).sealed);
  });

  it('reports progress and the missing parts', () => {
    const parts = splitKit(real);
    const asm = new PartAssembler();
    const r = asm.add(parts[1] as string);
    expect(r).toEqual({ status: 'progress', n: parts.length, have: [2], missing: parts.map((_, i) => i + 1).filter((i) => i !== 2) });
    expect(asm.add(parts[1] as string).status).toBe('progress'); // a repeat changes nothing
  });

  it('rejects a changed chunk (hash mismatch)', () => {
    const parts = splitKit(real);
    const bad = [...parts];
    const last = bad[bad.length - 1] as string;
    bad[bad.length - 1] = `${last.slice(0, -2)}${last.endsWith('A') ? 'B' : 'A'}${last.slice(-1)}`;
    expect(assembleKit(bad)).toBeNull();
  });

  it('keeps parts of two kits apart and never mixes them', () => {
    const other = kitQrText(padSealed(read('TEST_ONLY_owner_kit.zandglass'), 4200));
    const a = splitKit(real);
    const b = splitKit(other);
    expect(kitHash(real)).not.toBe(kitHash(other));
    const asm = new PartAssembler();
    for (const p of a.slice(0, -1)) expect(asm.add(p).status).toBe('progress');
    for (const p of b.slice(0, -1)) expect(asm.add(p).status).toBe('progress');
    expect(asm.add(a[a.length - 1] as string)).toEqual({ status: 'done', text: real });
    // a forged part: the other kit's hash but this kit's chunk
    const forged = (b[0] as string).replace(/^(ZGK1:1\/\d+:[0-9a-f]{8}:).*$/s, `$1${(a[0] as string).split(':').slice(3).join(':')}`);
    expect(assembleKit([forged, ...b.slice(1)])).toBeNull();
  });

  it('rejects malformed headers and plain text passes through', () => {
    expect(parsePart('ZGK1:0/3:abcdef12:x')).toBeNull();
    expect(parsePart('ZGK1:4/3:abcdef12:x')).toBeNull();
    expect(parsePart('ZGK1:1/3:ABCDEF12:x')).toBeNull();
    expect(new PartAssembler().add('ZGK1:garbage')).toEqual({ status: 'bad' });
    expect(new PartAssembler().add('{"kit":1}')).toEqual({ status: 'plain', text: '{"kit":1}' });
  });

  it('every part renders to pixels and decodes back with jsQR', async () => {
    const parts = splitKit(real);
    for (const p of parts) {
      const { data, size } = qrPixels(p, 4, PART_ECC);
      expect(await decodeQrPixels(data, size, size)).toBe(p);
    }
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
