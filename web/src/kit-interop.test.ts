/**
 * Interop: the committed kit vector (sealed by the Rust core) opens in
 * TypeScript with tlock-js + age-encryption, and a kit sealed in TypeScript
 * has the same layout. `scripts/interop-kit.sh` opens the TypeScript kit with
 * the reference CLIs and the Rust core (set KIT_OUT to write it to a file).
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { bech32 } from '@scure/base';
import { hkdf } from '@noble/hashes/hkdf.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { mnemonicToSeedSync, validateMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { Decrypter, Encrypter, identityToRecipient } from 'age-encryption';
import { quicknetClient, type ChainClient, type ChainInfo } from 'drand-client';
import { Buffer, timelockDecrypt, timelockEncrypt } from 'tlock-js';
import { describe, expect, it } from 'vitest';

const VECTORS = new URL('../../crates/sandglass-core/tests/vectors/', import.meta.url);
const read = (name: string): string => readFileSync(new URL(name, VECTORS), 'utf8');

const QUICKNET_INFO: ChainInfo = {
  public_key:
    '83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a',
  period: 3,
  genesis_time: 1692803367,
  hash: '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971',
  groupHash: 'f477d5c89f21a17c863a7f937c6a6d15859414d2be09cd448d4279af331c5d3e',
  schemeID: 'bls-unchained-g1-rfc9380',
  metadata: { beaconID: 'quicknet' },
};

/** A chain client that serves only the recorded beacon, so the test runs offline. */
function recordedClient(): ChainClient {
  const beacon = JSON.parse(read('beacon.json')) as { round: number };
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

/** phrase -> BIP 39 seed -> HKDF-SHA256("sandglass-heir-v1") -> AGE-SECRET-KEY-1... */
function identityFromPhrase(phrase: string): string {
  if (!validateMnemonic(phrase, wordlist)) throw new Error('invalid phrase');
  const seed = mnemonicToSeedSync(phrase, '');
  const secret = hkdf(sha256, seed, undefined, new TextEncoder().encode('sandglass-heir-v1'), 32);
  return bech32.encode('age-secret-key-', bech32.toWords(secret), 1023).toUpperCase();
}

async function openKit(kit: string, identity: string, client: ChainClient): Promise<string> {
  const inner = await timelockDecrypt(kit, client);
  const d = new Decrypter();
  d.addIdentity(identity);
  return d.decrypt(new Uint8Array(inner), 'text');
}

describe('kit interop (TypeScript)', () => {
  it('derives the same heir identity from the phrase as the Rust core', () => {
    expect(identityFromPhrase(read('TEST_ONLY_phrase.txt').trim())).toBe(
      read('TEST_ONLY_identity.txt').trim(),
    );
  });

  it('opens the Rust-sealed vector offline with the recorded beacon', async () => {
    const payload = await openKit(
      read('kit.txt'),
      read('TEST_ONLY_identity.txt').trim(),
      recordedClient(),
    );
    expect(payload).toBe(read('payload.txt'));
  });

  it.skipIf(!process.env['SANDGLASS_NETWORK_TESTS'])('opens the Rust-sealed vector with live drand relays', async () => {
    const payload = await openKit(read('kit.txt'), read('TEST_ONLY_identity.txt').trim(), quicknetClient() as ChainClient);
    expect(payload).toBe(read('payload.txt'));
  }, 30_000);

  it('rejects a wrong heir key', async () => {
    const other = identityFromPhrase(
      Array(23).fill('abandon').concat('art').join(' '),
    );
    await expect(openKit(read('kit.txt'), other, recordedClient())).rejects.toThrow();
  });

  it('seals a kit in TypeScript with the same layout', async () => {
    const identity = read('TEST_ONLY_identity.txt').trim();
    const round = Number(read('round.txt').trim());
    const payload = 'TEST ONLY payload sealed by TypeScript\n';

    const enc = new Encrypter();
    enc.addRecipient(await identityToRecipient(identity));
    const inner = await enc.encrypt(payload);
    const kit = await timelockEncrypt(round, Buffer.from(inner), recordedClient());

    expect(kit.startsWith('-----BEGIN AGE ENCRYPTED FILE-----')).toBe(true);
    expect(await openKit(kit, identity, recordedClient())).toBe(payload);
    const out = process.env['KIT_OUT'];
    if (out) writeFileSync(out, kit.endsWith('\n') ? kit : `${kit}\n`);
  });

  // Set KIT_IN to a kit sealed by another implementation (for example the
  // reference CLIs) to check that TypeScript can open it.
  it.skipIf(!process.env['KIT_IN'])('opens the kit given in KIT_IN', async () => {
    const kit = readFileSync(process.env['KIT_IN'] ?? '', 'utf8');
    const payload = await openKit(kit, read('TEST_ONLY_identity.txt').trim(), recordedClient());
    expect(payload).toBe(read('payload.txt'));
  });
});
