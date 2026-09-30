/**
 * Interop: the committed kit vector (sealed by the Rust core) opens in
 * TypeScript with tlock-js + age-encryption, and a kit sealed in TypeScript
 * has the same layout. `scripts/interop-kit.sh` opens the TypeScript kit with
 * the reference CLIs and the Rust core (set KIT_OUT to write it to a file).
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { identityFromPhrase } from './lib/heir';
import { read, recordedClient } from './test-helpers';
import { Decrypter, Encrypter, identityToRecipient } from 'age-encryption';
import { quicknetClient, type ChainClient } from 'drand-client';
import { Buffer, timelockDecrypt, timelockEncrypt } from 'tlock-js';
import { describe, expect, it } from 'vitest';


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
