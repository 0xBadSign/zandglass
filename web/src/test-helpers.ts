/** Shared helpers for tests: the committed vectors and an offline drand client. */
import { readFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import qrcode from 'qrcode-generator';
import { Encrypter, identityToRecipient } from 'age-encryption';
import type { ChainClient } from 'drand-client';
import { Buffer, timelockEncrypt } from 'tlock-js';
import { QUICKNET_INFO } from './lib/drand';

export const VECTORS = new URL('../../crates/zandglass-core/tests/vectors/', import.meta.url);

export const read = (name: string): string => readFileSync(new URL(name, VECTORS), 'utf8');

export interface RecordedBeacon {
  round: number;
  randomness: string;
  signature: string;
}

export const recordedBeacon = (): RecordedBeacon => JSON.parse(read('beacon.json')) as RecordedBeacon;

/** A chain client that serves only the recorded beacon, so tests run offline. */
export function recordedClient(): ChainClient {
  const beacon = recordedBeacon();
  return {
    options: { disableBeaconVerification: true, noCache: true },
    latest: async () => beacon as never,
    get: async (round: number) => {
      if (round !== beacon.round) throw new Error(`no recorded beacon for round ${round}`);
      return beacon as never;
    },
    chain: () => ({ baseUrl: 'recorded', info: async () => QUICKNET_INFO }),
  };
}

/** Render a QR code to RGBA pixels (black on white, quiet zone, `scale` px per module). */
export function qrPixels(text: string, scale = 4): { data: Uint8ClampedArray; size: number } {
  const qr = qrcode(0, 'L');
  qr.addData(text);
  qr.make();
  const n = qr.getModuleCount();
  const size = (n + 8) * scale;
  const data = new Uint8ClampedArray(size * size * 4).fill(255);
  for (let r = 0; r < n; r++) {
    for (let c = 0; c < n; c++) {
      if (!qr.isDark(r, c)) continue;
      for (let y = 0; y < scale; y++) {
        for (let x = 0; x < scale; x++) {
          const i = (((r + 4) * scale + y) * size + (c + 4) * scale + x) * 4;
          data[i] = data[i + 1] = data[i + 2] = 0;
        }
      }
    }
  }
  return { data, size };
}

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(buf: Buffer): number {
  let c = 0xffffffff;
  for (const b of buf) c = (CRC_TABLE[(c ^ b) & 0xff] as number) ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, body: Buffer): Buffer {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(body.length);
  const td = Buffer.concat([Buffer.from(type, 'ascii'), body]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(td));
  return Buffer.concat([len, td, crc]);
}

/** A grayscale PNG of the QR code for `text` (a stand-in for a photo of a paper kit). */
export function qrPng(text: string, scale = 4): Buffer {
  const { data, size } = qrPixels(text, scale);
  const raw = Buffer.alloc((size + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size + 1)] = 0;
    for (let x = 0; x < size; x++) raw[y * (size + 1) + 1 + x] = data[(y * size + x) * 4] as number;
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 0; // grayscale
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', ihdr),
    pngChunk('IDAT', deflateSync(raw)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/** A test payload with a clearly fake 24-word seed (PLAN 5.4 payload layout). */
export const TEST_PAYLOAD = {
  vault_seed:
    'zoo wrap quiz pond ocean lunar kiwi jelly index hover glad fossil ember delta cabin balance ankle share mule orbit vivid tunnel ribbon sphere',
  birthday_height: 3500000,
  network: 'test',
  plan: '0123456789abcdef0123456789abcdef',
  epoch: 3,
  message: 'TEST ONLY: look after each other.',
  restore_hints: ['TEST ONLY hint: restore in any wallet.'],
};

/** Seal `payload` to the TEST_ONLY heir and the recorded round, as a JSON kit. */
export async function makeJsonKit(payload: object = TEST_PAYLOAD): Promise<string> {
  const enc = new Encrypter();
  enc.addRecipient(await identityToRecipient(read('TEST_ONLY_identity.txt').trim()));
  const inner = await enc.encrypt(JSON.stringify(payload));
  const round = recordedBeacon().round;
  const sealed = await timelockEncrypt(round, Buffer.from(inner), recordedClient());
  return JSON.stringify({
    format: 'zandglass-kit',
    version: 1,
    kind: 'legacy',
    network: 'test',
    created: '2026-10-01T00:00:00Z',
    plan: '0123456789abcdef0123456789abcdef',
    epoch: 3,
    unlock: { drand_chain: QUICKNET_INFO.hash, round, time: '2026-10-01T00:00:00Z' },
    sealed,
  });
}
