import { beforeEach, describe, expect, it } from 'vitest';
import { read } from '../test-helpers';
import { parseKit } from './kit';
import {
  OwnerSigInvalidError,
  checkOwner,
  ownerFingerprint,
  ownerSigMessage,
  pinOwnerKey,
  pinnedOwnerKey,
  repinOwnerKey,
  verifyOwnerSigOf,
} from './owner-sig';

// The vector is produced by crates/zandglass-core/tests/owner_sig.rs.
const kitText = read('TEST_ONLY_owner_kit.zandglass');
const keyFile = read('TEST_ONLY_owner_key.txt');
const PUBLIC = /^public=([0-9a-f]{64})$/m.exec(keyFile)?.[1] ?? '';
const FINGERPRINT = /^fingerprint=(\S+)$/m.exec(keyFile)?.[1] ?? '';
const PLAN = '00112233445566778899aabbccddeeff';

function mutated(edit: (k: Record<string, unknown>) => void): string {
  const k = JSON.parse(kitText) as Record<string, unknown>;
  edit(k);
  return JSON.stringify(k);
}

function memoryStorage(): void {
  const data = new Map<string, string>();
  (globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
  };
}

describe('owner signature (cross-language vector)', () => {
  it('builds the same canonical message as the Rust core', () => {
    const raw = JSON.parse(kitText) as Record<string, unknown>;
    const msg = new TextDecoder().decode(ownerSigMessage(raw, PUBLIC));
    expect(msg).toBe(read('TEST_ONLY_owner_message.txt'));
  });

  it('verifies the Rust-signed kit and reports the key', async () => {
    const r = await verifyOwnerSigOf(parseKit(kitText));
    expect(r).toEqual({ fingerprint: FINGERPRINT, key: PUBLIC });
    expect(ownerFingerprint(PUBLIC)).toBe(FINGERPRINT);
  });

  it('treats a kit without owner fields as unsigned, also as a bare armour', async () => {
    const stripped = mutated((k) => {
      delete k['owner_key'];
      delete k['owner_sig'];
    });
    expect(await verifyOwnerSigOf(parseKit(stripped))).toBe('unsigned');
    expect(await verifyOwnerSigOf(parseKit(read('kit.txt')))).toBe('unsigned');
  });

  it('rejects tampering with any signed field or the sealed part', async () => {
    const edits: Array<[string, (k: Record<string, unknown>) => void]> = [
      ['epoch', (k) => void (k['epoch'] = 3)],
      ['plan', (k) => void (k['plan'] = 'f'.repeat(32))],
      ['created', (k) => void (k['created'] = '2026-01-01T00:00:00Z')],
      ['network', (k) => void (k['network'] = 'main')],
      ['kind', (k) => void (k['kind'] = 'legacy')],
      ['sequence', (k) => void delete k['sequence']],
      ['sealed', (k) => void (k['sealed'] = String(k['sealed']).replace('YWdl', 'YWdm'))],
      ['key', (k) => void (k['owner_key'] = 'a'.repeat(64))],
    ];
    for (const [name, edit] of edits) {
      let err: unknown;
      try {
        await verifyOwnerSigOf(parseKit(mutated(edit)));
      } catch (e) {
        err = e;
      }
      // `sealed` may already fail in parseKit as a damaged kit; both refuse.
      expect(err, name).toBeInstanceOf(Error);
      if (name !== 'sealed') expect(err, name).toBeInstanceOf(OwnerSigInvalidError);
    }
  });

  it('rejects incomplete or malformed owner fields', async () => {
    for (const edit of [
      (k: Record<string, unknown>) => void delete k['owner_sig'],
      (k: Record<string, unknown>) => void delete k['owner_key'],
      (k: Record<string, unknown>) => void (k['owner_sig'] = 'ab'),
      (k: Record<string, unknown>) => void (k['owner_key'] = 'AB'.repeat(32)),
      (k: Record<string, unknown>) => void (k['owner_sig'] = 5),
    ]) {
      await expect(verifyOwnerSigOf(parseKit(mutated(edit)))).rejects.toBeInstanceOf(
        OwnerSigInvalidError,
      );
    }
  });
});

describe('owner key pinning', () => {
  beforeEach(memoryStorage);

  it('remembers the first key per plan', () => {
    expect(pinnedOwnerKey(PLAN)).toBeNull();
    expect(pinOwnerKey(PLAN, PUBLIC)).toBe('new');
    expect(pinOwnerKey(PLAN, PUBLIC)).toBe('match');
    const other = 'b'.repeat(64);
    expect(pinOwnerKey(PLAN, other)).toBe('mismatch');
    expect(pinnedOwnerKey(PLAN)).toBe(PUBLIC);
    expect(pinOwnerKey('1'.repeat(32), other)).toBe('new');
    repinOwnerKey(PLAN, other);
    expect(pinOwnerKey(PLAN, other)).toBe('match');
  });

  it('works without localStorage', () => {
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(pinOwnerKey(PLAN, PUBLIC)).toBe('new');
    expect(pinOwnerKey(PLAN, PUBLIC)).toBe('new');
  });

  it('checkOwner: new, match, mismatch and stripped signatures', async () => {
    const signed = parseKit(kitText);
    expect(await checkOwner(signed)).toEqual({ status: 'new', fingerprint: FINGERPRINT });
    expect(await checkOwner(signed)).toEqual({ status: 'match', fingerprint: FINGERPRINT });

    const unsigned = parseKit(
      mutated((k) => {
        delete k['owner_key'];
        delete k['owner_sig'];
      }),
    );
    expect(await checkOwner(unsigned)).toEqual({
      status: 'stripped',
      pinnedFingerprint: FINGERPRINT,
    });

    // A decoy signed by another key (made here with a different pin) mismatches.
    repinOwnerKey(PLAN, 'c'.repeat(64));
    const r = await checkOwner(signed);
    expect(r.status).toBe('mismatch');
    expect(pinnedOwnerKey(PLAN)).toBe('c'.repeat(64));
  });

  it('checkOwner: unsigned with no pin is simply unsigned', async () => {
    const unsigned = parseKit(
      mutated((k) => {
        delete k['owner_key'];
        delete k['owner_sig'];
      }),
    );
    expect(await checkOwner(unsigned)).toEqual({ status: 'unsigned' });
  });
});
