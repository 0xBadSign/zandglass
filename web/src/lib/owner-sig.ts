/**
 * Owner-authenticated kits (docs/KIT_FORMAT.md section 8).
 *
 * A plan's kits may carry `owner_key` (Ed25519 public key, 64 lowercase hex)
 * and `owner_sig` (128 hex) over a canonical text message of the kit's public
 * part. The heir page pins the first key it sees for a plan and flags later
 * kits that are unsigned or signed by another key. Everything runs in the
 * browser; the pin is a per-browser convenience kept in localStorage.
 */
import { sha256 } from '@noble/hashes/sha2.js';
import { ed25519 } from '@noble/curves/ed25519.js';
import type { LoadedKit } from './kit';

/** A kit carries owner fields that do not verify (tampered, or not the owner's). */
export class OwnerSigInvalidError extends Error {
  constructor(message = 'the owner signature of this kit is invalid') {
    super(message);
    this.name = 'OwnerSigInvalidError';
  }
}

export const SIG_DOMAIN = 'zandglass-kit-sig-v1';

export interface OwnerKey {
  /** Short comparable form of the key, `xxxx-xxxx-xxxx-xxxx`. */
  fingerprint: string;
  /** The full public key, 64 lowercase hex. Pin this, not the fingerprint. */
  key: string;
}

const HEX_KEY = /^[0-9a-f]{64}$/;
const HEX_SIG = /^[0-9a-f]{128}$/;

function toHex(bytes: Uint8Array): string {
  let s = '';
  for (const b of bytes) s += b.toString(16).padStart(2, '0');
  return s;
}

function fromHex(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

/** `xxxx-xxxx-xxxx-xxxx`: the first eight bytes of SHA-256 of the public key. */
export function ownerFingerprint(keyHex: string): string {
  const h = toHex(sha256(fromHex(keyHex)).slice(0, 8));
  return `${h.slice(0, 4)}-${h.slice(4, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}`;
}

function text(v: unknown): string {
  if (typeof v !== 'string' || v.includes('\n')) throw new OwnerSigInvalidError();
  return v;
}

function int(v: unknown): string {
  if (typeof v !== 'number' || !Number.isSafeInteger(v)) throw new OwnerSigInvalidError();
  return String(v);
}

/**
 * The exact bytes that were signed. Same fixed field order as the Rust
 * `zandglass_core::owner::signing_message`; the committed vector
 * `TEST_ONLY_owner_message.txt` pins both.
 */
export function ownerSigMessage(kit: Record<string, unknown>, ownerKey: string): Uint8Array {
  const unlock = kit['unlock'];
  if (typeof unlock !== 'object' || unlock === null) throw new OwnerSigInvalidError();
  const u = unlock as Record<string, unknown>;
  let sequence = 'none';
  if (kit['sequence'] !== undefined) {
    const s = kit['sequence'];
    if (typeof s !== 'object' || s === null) throw new OwnerSigInvalidError();
    const q = s as Record<string, unknown>;
    sequence = `${int(q['index'])}/${int(q['count'])}`;
  }
  const sealed = kit['sealed'];
  if (typeof sealed !== 'string') throw new OwnerSigInvalidError();
  const sealedHash = toHex(sha256(new TextEncoder().encode(sealed)));
  const lines = [
    SIG_DOMAIN,
    `format=${text(kit['format'])}`,
    `version=${int(kit['version'])}`,
    `kind=${text(kit['kind'])}`,
    `network=${text(kit['network'])}`,
    `created=${text(kit['created'])}`,
    `plan=${text(kit['plan'])}`,
    `epoch=${int(kit['epoch'])}`,
    `sequence=${sequence}`,
    `unlock.drand_chain=${text(u['drand_chain'])}`,
    `unlock.round=${int(u['round'])}`,
    `unlock.time=${text(u['time'])}`,
    `owner_key=${ownerKey}`,
    `sealed_sha256=${sealedHash}`,
  ];
  return new TextEncoder().encode(lines.join('\n') + '\n');
}

/**
 * Verify the owner signature of a loaded kit. Returns `'unsigned'` when the
 * kit has no owner fields (or is a bare armour string), the owner key when the
 * signature verifies, and throws `OwnerSigInvalidError` when owner fields are
 * present but wrong or incomplete. Offline; no network, no secrets.
 */
export async function verifyOwnerSigOf(kit: LoadedKit): Promise<'unsigned' | OwnerKey> {
  const raw = kit.raw;
  if (!raw) return 'unsigned';
  const key = raw['owner_key'];
  const sig = raw['owner_sig'];
  if (key === undefined && sig === undefined) return 'unsigned';
  if (typeof key !== 'string' || typeof sig !== 'string') throw new OwnerSigInvalidError();
  if (!HEX_KEY.test(key) || !HEX_SIG.test(sig)) throw new OwnerSigInvalidError();
  const message = ownerSigMessage(raw, key);
  let ok = false;
  try {
    ok = ed25519.verify(fromHex(sig), message, fromHex(key), { zip215: false });
  } catch {
    ok = false;
  }
  if (!ok) throw new OwnerSigInvalidError();
  return { fingerprint: ownerFingerprint(key), key };
}

// ---- pinning -----------------------------------------------------------

const PIN_PREFIX = 'zandglass.ownerkey.v1.';

/** The key pinned for a plan in this browser, or null. */
export function pinnedOwnerKey(plan: string): string | null {
  try {
    const v = localStorage.getItem(PIN_PREFIX + plan);
    return v !== null && HEX_KEY.test(v) ? v : null;
  } catch {
    return null;
  }
}

/**
 * Remember the first owner key seen for a plan (full public key, hex).
 * `'new'` the first time (and it is stored), `'match'` for the same key,
 * `'mismatch'` for another one (the pin is left unchanged). Without usable
 * localStorage every call answers `'new'`.
 */
export function pinOwnerKey(plan: string, ownerKey: string): 'new' | 'match' | 'mismatch' {
  const pinned = pinnedOwnerKey(plan);
  if (pinned === null) {
    try {
      localStorage.setItem(PIN_PREFIX + plan, ownerKey);
    } catch {
      // private window or blocked storage: nothing to remember
    }
    return 'new';
  }
  return pinned === ownerKey ? 'match' : 'mismatch';
}

/** Replace the pin after the person decided to trust the new key. */
export function repinOwnerKey(plan: string, ownerKey: string): void {
  try {
    localStorage.setItem(PIN_PREFIX + plan, ownerKey);
  } catch {
    // see pinOwnerKey
  }
}

export type OwnerCheck =
  /** No owner fields and nothing pinned: kits of this plan were never seen signed. */
  | { status: 'unsigned' }
  /** First signed kit of this plan seen in this browser; the key is now pinned. */
  | { status: 'new'; fingerprint: string }
  /** Signed by the pinned key. */
  | { status: 'match'; fingerprint: string }
  /** Signed, but by another key than the pinned one. */
  | { status: 'mismatch'; fingerprint: string; pinnedFingerprint: string }
  /** Unsigned, although an earlier kit of this plan was signed (signature stripped?). */
  | { status: 'stripped'; pinnedFingerprint: string };

/**
 * Verify and pin in one call: what the heir page needs for its banner.
 * Throws `OwnerSigInvalidError` for a present but invalid signature.
 */
export async function checkOwner(kit: LoadedKit): Promise<OwnerCheck> {
  const plan = kit.meta?.plan ?? null;
  const sig = await verifyOwnerSigOf(kit);
  if (sig === 'unsigned') {
    const pinned = plan ? pinnedOwnerKey(plan) : null;
    return pinned ? { status: 'stripped', pinnedFingerprint: ownerFingerprint(pinned) } : { status: 'unsigned' };
  }
  if (!plan) return { status: 'new', fingerprint: sig.fingerprint };
  const pinnedBefore = pinnedOwnerKey(plan);
  const state = pinOwnerKey(plan, sig.key);
  if (state === 'mismatch' && pinnedBefore) {
    return {
      status: 'mismatch',
      fingerprint: sig.fingerprint,
      pinnedFingerprint: ownerFingerprint(pinnedBefore),
    };
  }
  return state === 'match'
    ? { status: 'match', fingerprint: sig.fingerprint }
    : { status: 'new', fingerprint: sig.fingerprint };
}
