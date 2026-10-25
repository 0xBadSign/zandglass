import { createHash } from 'node:crypto';
import { beforeEach, describe, expect, it } from 'vitest';
import { read } from '../test-helpers';
import { parseKit } from './kit';
import {
  OwnerLinkError,
  OwnerSigInvalidError,
  checkOwner,
  ownerFingerprint,
  ownerKeyHint,
  ownerSigMessage,
  parseClaimFragment,
  pinOwnerKey,
  pinnedMailbox,
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

describe('claim link owner hint', () => {
  beforeEach(memoryStorage);

  const unsignedKit = (plan = PLAN) =>
    parseKit(
      mutated((k) => {
        delete k['owner_key'];
        delete k['owner_sig'];
        k['plan'] = plan;
      }),
    );
  const HINT = createHash('sha256').update(Buffer.from(PUBLIC, 'hex')).digest('hex').slice(0, 32);

  it('is the first 16 bytes of the key hash, as the Rust core builds it', () => {
    expect(ownerKeyHint(PUBLIC)).toBe(HINT);
    expect(HINT).toMatch(/^[0-9a-f]{32}$/);
  });

  it('parses claim fragments, with and without the hint', () => {
    expect(parseClaimFragment('')).toBeNull();
    expect(parseClaimFragment('#')).toBeNull();
    expect(parseClaimFragment('#m=abc&r=def')).toEqual({ id: 'abc', token: 'def', hint: null, keeper: null });
    expect(parseClaimFragment(`#m=abc&r=def&o=${HINT}`)).toEqual({ id: 'abc', token: 'def', hint: HINT, keeper: null });
    expect(parseClaimFragment(`#m=abc&r=def&o=${HINT}&k=http%3A%2F%2F127.0.0.1%3A1`)).toMatchObject({
      hint: HINT,
      keeper: 'http://127.0.0.1:1',
    });
    expect(parseClaimFragment('#m=abc')).toBe('incomplete');
    expect(parseClaimFragment('#r=def')).toBe('incomplete');
    expect(parseClaimFragment(`#o=${HINT}`)).toBe('incomplete');
  });

  it('refuses a malformed hint instead of ignoring it', () => {
    for (const bad of ['', 'abc', HINT.toUpperCase(), HINT + '0', 'g'.repeat(32)]) {
      expect(parseClaimFragment(`#m=abc&r=def&o=${bad}`)).toBe('incomplete');
    }
  });

  it('accepts a kit signed by the key the link names', async () => {
    const r = await checkOwner(parseKit(kitText), { hint: HINT, mailbox: 'mb1' });
    expect(r).toEqual({ status: 'link', fingerprint: FINGERPRINT });
  });

  it('refuses a kit signed by another key', async () => {
    await expect(checkOwner(parseKit(kitText), { hint: 'f'.repeat(32), mailbox: 'mb1' })).rejects.toMatchObject({
      name: 'OwnerLinkError',
      reason: 'mismatch',
    });
    // Nothing was pinned from the refused kit.
    expect(pinnedOwnerKey(PLAN)).toBeNull();
    expect(pinnedMailbox('mb1')).toBeNull();
  });

  it('refuses an unsigned kit when the link names a key', async () => {
    const p = checkOwner(unsignedKit(), { hint: HINT, mailbox: 'mb1' });
    await expect(p).rejects.toBeInstanceOf(OwnerLinkError);
    await expect(p).rejects.toMatchObject({ reason: 'unsigned' });
  });

  it('a kit from a mailbox without a hint: unsigned is a warning state, a file is not', async () => {
    expect(await checkOwner(unsignedKit(), { mailbox: 'mb1' })).toEqual({ status: 'unsigned-mailbox' });
    expect(await checkOwner(unsignedKit())).toEqual({ status: 'unsigned' });
  });

  it('pins by mailbox id too', async () => {
    const real = parseKit(kitText);
    expect(await checkOwner(real, { mailbox: 'mb1' })).toEqual({ status: 'new', fingerprint: FINGERPRINT });
    expect(pinnedMailbox('mb1')).toEqual({ plan: PLAN, key: PUBLIC });
    expect(await checkOwner(real, { mailbox: 'mb1' })).toEqual({ status: 'match', fingerprint: FINGERPRINT });

    // A decoy with a fresh plan id but no signature is a stripped signature for this mailbox.
    expect(await checkOwner(unsignedKit('1'.repeat(32)), { mailbox: 'mb1' })).toEqual({
      status: 'stripped',
      pinnedFingerprint: FINGERPRINT,
    });

    // Another key on a mailbox that served a different one: mismatch, and the pin does not move.
    repinOwnerKey('2'.repeat(32), 'c'.repeat(64), 'mb2');
    const r = await checkOwner(real, { mailbox: 'mb2' });
    expect(r.status).toBe('mismatch');
    expect(pinnedMailbox('mb2')?.key).toBe('c'.repeat(64));
  });

  it('flags the same key under another plan id on a pinned mailbox', async () => {
    repinOwnerKey('3'.repeat(32), PUBLIC, 'mb3');
    const r = await checkOwner(parseKit(kitText), { mailbox: 'mb3' });
    expect(r).toMatchObject({ status: 'mismatch', planChanged: true, pinnedFingerprint: FINGERPRINT });
  });

  it('works without localStorage', async () => {
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(await checkOwner(parseKit(kitText), { mailbox: 'mb9' })).toEqual({ status: 'new', fingerprint: FINGERPRINT });
    expect(await checkOwner(parseKit(kitText), { hint: HINT, mailbox: 'mb9' })).toMatchObject({ status: 'link' });
  });
});
