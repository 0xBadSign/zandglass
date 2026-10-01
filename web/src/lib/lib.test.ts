import { describe, expect, it } from 'vitest';
import { BeaconNotReadyError, NetworkError, relayClient } from './drand';
import { generatePhrase, heirCodeFromPhrase, identityFromPhrase, isValidPhrase } from './heir';
import { KitDamagedError, WrongHeirKeyError, armorRound, openKit, parseKit } from './kit';
import { TEST_PAYLOAD, makeJsonKit, read, recordedBeacon, recordedClient } from '../test-helpers';

const phrase = read('TEST_ONLY_phrase.txt').trim();
const CHAIN = '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971';

describe('heir keys', () => {
  it('generates valid 24-word phrases', () => {
    const p = generatePhrase();
    expect(p.split(' ')).toHaveLength(24);
    expect(isValidPhrase(p)).toBe(true);
    expect(generatePhrase()).not.toBe(p);
  });

  it('derives the committed heir code', async () => {
    expect(await heirCodeFromPhrase(phrase)).toBe(read('heir_code.txt').trim());
    expect(await heirCodeFromPhrase(`  ${phrase.toUpperCase()}\n`)).toBe(read('heir_code.txt').trim());
  });

  it('rejects short or misspelt phrases', () => {
    expect(() => identityFromPhrase('abandon abandon')).toThrow();
    expect(isValidPhrase(phrase.replace('drastic', 'drasticc'))).toBe(false);
  });
});

describe('kit parsing', () => {
  it('reads the round and chain from a bare armour string', () => {
    expect(armorRound(read('kit.txt'))).toEqual({
      round: Number(read('round.txt').trim()),
      chain: CHAIN,
    });
  });

  it('parses a JSON kit and rejects damaged ones', async () => {
    const json = await makeJsonKit();
    const k = parseKit(json);
    expect(k.round).toBe(recordedBeacon().round);
    expect(k.meta?.network).toBe('test');
    expect(k.meta?.epoch).toBe(3);
    expect(() => parseKit(json.slice(0, json.length - 40))).toThrow(KitDamagedError);
    expect(() => parseKit('{}')).toThrow(KitDamagedError);
    expect(() => parseKit(json.replace('"version":1', '"version":2'))).toThrow(KitDamagedError);
    expect(() => parseKit('hello')).toThrow(KitDamagedError);
  });
});

describe('opening kits', () => {
  it('opens the Rust vector (bare armour, raw payload)', async () => {
    const r = await openKit(parseKit(read('kit.txt')), phrase, recordedClient());
    expect(r).toEqual({ kind: 'raw', text: read('payload.txt') });
  });

  it('opens a JSON kit with a JSON payload', async () => {
    const r = await openKit(parseKit(await makeJsonKit()), phrase, recordedClient());
    expect(r.kind).toBe('json');
    if (r.kind === 'json') {
      expect(r.payload.vault_seed).toBe(TEST_PAYLOAD.vault_seed);
      expect(r.payload.birthday_height).toBe(3500000);
    }
  });

  it('reports a wrong heir key', async () => {
    const other = generatePhrase();
    await expect(openKit(parseKit(read('kit.txt')), other, recordedClient())).rejects.toThrow(
      WrongHeirKeyError,
    );
  });

  it('reports a damaged inner layer', async () => {
    const lines = read('kit.txt').trim().split('\n');
    const i = lines.length - 3;
    const line = lines[i] ?? '';
    lines[i] = (line.startsWith('A') ? 'B' : 'A') + line.slice(1);
    await expect(openKit(parseKit(lines.join('\n')), phrase, recordedClient())).rejects.toThrow(
      KitDamagedError,
    );
  });
});

describe('relay client', () => {
  const beacon = recordedBeacon();
  const ok = () => new Response(JSON.stringify(beacon), { status: 200 });

  it('uses another relay when one fails', async () => {
    const urls: string[] = [];
    const f = (async (u: string) => {
      urls.push(u);
      if (urls.length === 1) throw new TypeError('offline');
      return ok();
    }) as unknown as typeof fetch;
    const c = relayClient(['https://a.test', 'https://b.test'], f);
    expect(await c.get(beacon.round)).toEqual(beacon);
    expect(urls[1]).toBe(`https://b.test/${CHAIN}/public/${beacon.round}`);
  });

  it('distinguishes not-yet-published from network errors', async () => {
    const notFound = (async () => new Response('', { status: 404 })) as unknown as typeof fetch;
    await expect(relayClient(['https://a.test'], notFound).get(1)).rejects.toThrow(BeaconNotReadyError);
    const down = (async () => {
      throw new TypeError('offline');
    }) as unknown as typeof fetch;
    await expect(relayClient(['https://a.test'], down).get(1)).rejects.toThrow(NetworkError);
  });
});
