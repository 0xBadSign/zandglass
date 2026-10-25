/**
 * Kits (PLAN 5.4): public JSON metadata around a `sealed` tlock armour string.
 * sealed = tlock_R( age_to(heir_codes, payload) ). Everything here runs in the
 * browser; the phrase and the decrypted payload never leave it.
 */
import { Decrypter, Encrypter } from 'age-encryption';
import type { ChainClient } from 'drand-client';
import { Buffer, timelockDecrypt, timelockEncrypt } from 'tlock-js';
import { BeaconNotReadyError, NetworkError, QUICKNET_INFO } from './drand';
import { identityFromPhrase } from './heir';
import { roundTime, unlockRound } from '../drand';
import { verifyOwnerSigOf, type OwnerKey } from './owner-sig';

export {
  OwnerSigInvalidError,
  checkOwner,
  ownerFingerprint,
  pinOwnerKey,
  pinnedOwnerKey,
  repinOwnerKey,
  type OwnerCheck,
  type OwnerKey,
} from './owner-sig';

/**
 * Verify the kit's owner signature (docs/KIT_FORMAT.md section 8): `'unsigned'`
 * when the kit has no owner fields, otherwise the owner key's fingerprint
 * (plus the full key). Throws `OwnerSigInvalidError` when the signature is
 * present but wrong.
 */
export async function verifyOwnerSig(kit: LoadedKit): Promise<'unsigned' | OwnerKey> {
  return verifyOwnerSigOf(kit);
}

export class KitDamagedError extends Error {
  constructor(message = 'kit damaged') {
    super(message);
    this.name = 'KitDamagedError';
  }
}

export class WrongHeirKeyError extends Error {
  constructor(message = 'wrong heir key') {
    super(message);
    this.name = 'WrongHeirKeyError';
  }
}

export interface KitMeta {
  kind: string;
  network: string;
  created: string | null;
  plan: string | null;
  epoch: number | null;
  unlockTime: string | null;
}

export interface LoadedKit {
  /** The tlock armour string. */
  sealed: string;
  round: number;
  chain: string;
  /** Public metadata; null when the file is a bare armour string. */
  meta: KitMeta | null;
  /** The parsed JSON object of a JSON kit (for owner signature checks); absent for a bare armour. */
  raw?: Record<string, unknown>;
}

export interface Payload {
  vault_seed: string;
  birthday_height: number;
  network?: string;
  plan?: string;
  epoch?: number;
  message?: string;
  restore_hints?: string[];
  /** True only for a practice kit (heir fire drill): it holds no seed. */
  practice?: boolean;
}

export type OpenedPayload =
  | { kind: 'json'; payload: Payload }
  | { kind: 'raw'; text: string };

const BEGIN = '-----BEGIN AGE ENCRYPTED FILE-----';
const END = '-----END AGE ENCRYPTED FILE-----';

/** Read the tlock stanza (`-> tlock <round> <chain>`) from an armoured age file. */
export function armorRound(armored: string): { round: number; chain: string } {
  const trimmed = armored.trim();
  if (!trimmed.startsWith(BEGIN) || !trimmed.endsWith(END)) throw new KitDamagedError();
  const body = trimmed.slice(BEGIN.length, trimmed.length - END.length).replace(/\s+/g, '');
  let header: string;
  try {
    const bytes = atob(body.slice(0, 400).slice(0, Math.floor(Math.min(body.length, 400) / 4) * 4));
    header = bytes.split('\n').slice(0, 3).join('\n');
  } catch {
    throw new KitDamagedError();
  }
  const m = /^-> tlock (\d+) ([0-9a-f]{64})$/m.exec(header);
  if (!m) throw new KitDamagedError();
  const round = Number(m[1]);
  if (!Number.isSafeInteger(round) || round < 1) throw new KitDamagedError();
  return { round, chain: m[2] ?? '' };
}

function str(v: unknown): string | null {
  return typeof v === 'string' ? v : null;
}

/** Parse a `*.zandglass` JSON kit, or a bare tlock armour string. */
export function parseKit(text: string): LoadedKit {
  const t = text.replace(/^﻿/, '').trim();
  if (t.startsWith(BEGIN)) {
    const { round, chain } = armorRound(t);
    return { sealed: t, round, chain, meta: null };
  }
  let json: unknown;
  try {
    json = JSON.parse(t);
  } catch {
    throw new KitDamagedError();
  }
  if (typeof json !== 'object' || json === null) throw new KitDamagedError();
  const k = json as Record<string, unknown>;
  if (k['format'] !== 'zandglass-kit') throw new KitDamagedError();
  if (k['version'] !== 1) throw new KitDamagedError('unsupported kit version');
  const sealed = str(k['sealed']);
  if (!sealed) throw new KitDamagedError();
  const { round, chain } = armorRound(sealed);
  const unlock = (k['unlock'] ?? {}) as Record<string, unknown>;
  if (unlock['round'] !== undefined && unlock['round'] !== round) throw new KitDamagedError();
  if (typeof unlock['drand_chain'] === 'string' && unlock['drand_chain'] !== chain) {
    throw new KitDamagedError();
  }
  const epoch = k['epoch'];
  return {
    sealed,
    round,
    chain,
    raw: k,
    meta: {
      kind: str(k['kind']) ?? 'legacy',
      network: str(k['network']) ?? 'main',
      created: str(k['created']),
      plan: str(k['plan']),
      epoch: typeof epoch === 'number' ? epoch : null,
      unlockTime: str(unlock['time']),
    },
  };
}

/** Unix seconds at which the kit unlocks, from its round (authoritative). */
export function unlockSeconds(kit: LoadedKit): number {
  return roundTime(kit.round);
}

/** Open both layers with the heir phrase. Throws the typed errors above. */
export async function openKit(
  kit: LoadedKit,
  phrase: string,
  client: ChainClient,
): Promise<OpenedPayload> {
  const identity = identityFromPhrase(phrase);
  let inner: Buffer;
  try {
    inner = await timelockDecrypt(kit.sealed, client);
  } catch (e) {
    if (e instanceof NetworkError || e instanceof BeaconNotReadyError) throw e;
    throw new KitDamagedError();
  }
  let text: string;
  try {
    const d = new Decrypter();
    d.addIdentity(identity);
    text = await d.decrypt(new Uint8Array(inner), 'text');
  } catch (e) {
    const msg = e instanceof Error ? e.message : '';
    if (/no identity matched|recipient/i.test(msg)) throw new WrongHeirKeyError();
    throw new KitDamagedError();
  }
  const opened = parsePayload(text);
  // The sealed payload must describe the same plan as the public part
  // (the Rust reader checks the same), so a kit cannot be relabelled.
  if (opened.kind === 'json' && kit.meta) {
    const p = opened.payload;
    if (
      (p.network !== undefined && p.network !== kit.meta.network) ||
      (p.plan !== undefined && kit.meta.plan !== null && p.plan !== kit.meta.plan) ||
      (p.epoch !== undefined && kit.meta.epoch !== null && p.epoch !== kit.meta.epoch)
    ) {
      throw new KitDamagedError('the sealed payload does not match the kit');
    }
  }
  return opened;
}

export function parsePayload(text: string): OpenedPayload {
  try {
    const p = JSON.parse(text) as Record<string, unknown>;
    if (typeof p['vault_seed'] === 'string') {
      const hints = Array.isArray(p['restore_hints'])
        ? p['restore_hints'].filter((h): h is string => typeof h === 'string')
        : undefined;
      const payload: Payload = {
        vault_seed: p['vault_seed'],
        birthday_height: typeof p['birthday_height'] === 'number' ? p['birthday_height'] : 0,
      };
      if (typeof p['network'] === 'string') payload.network = p['network'];
      if (typeof p['plan'] === 'string') payload.plan = p['plan'];
      if (typeof p['epoch'] === 'number') payload.epoch = p['epoch'];
      if (typeof p['message'] === 'string' && p['message']) payload.message = p['message'];
      if (hints && hints.length) payload.restore_hints = hints;
      if (p['practice'] === true) payload.practice = true;
      return { kind: 'json', payload };
    }
  } catch {
    // not JSON: fall through and show the text as it is
  }
  return { kind: 'raw', text };
}

/** sessionStorage key used to hand a practice kit from /heir to /claim (same origin). */
export const PRACTICE_HANDOFF_KEY = 'zandglass-practice-kit';

/** Seconds between sealing a practice kit and its unlock. */
export const PRACTICE_DELAY_SECONDS = 60;

function randomHex(bytes: number): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (b) => b.toString(16).padStart(2, '0')).join('');
}

/** A client that only knows the pinned quicknet chain info: sealing needs no network. */
const sealClient = {
  options: { disableBeaconVerification: true, noCache: true },
  latest: async () => {
    throw new Error('latest is not supported');
  },
  get: async () => {
    throw new Error('get is not supported');
  },
  chain: () => ({ baseUrl: '', info: async () => QUICKNET_INFO }),
} as unknown as ChainClient;

/** Seal `payload` to the heir codes and `round`: tlock( age_to(heirCodes, payload) ). */
export async function sealPayload(payload: object, heirCodes: string[], round: number): Promise<string> {
  const enc = new Encrypter();
  for (const code of heirCodes) enc.addRecipient(code);
  const inner = await enc.encrypt(JSON.stringify(payload));
  return timelockEncrypt(round, Buffer.from(inner), sealClient);
}

export interface PracticeKit {
  /** The kit file text (JSON). */
  text: string;
  /** Suggested file name: practice-<short>.zandglass */
  fileName: string;
  round: number;
}

/**
 * A PRACTICE kit sealed to the heir's own code, unlocking about a minute from
 * `nowMs`. The payload holds no seed (empty vault_seed) and `practice: true`.
 */
export async function sealPracticeKit(heirCode: string, nowMs: number = Date.now()): Promise<PracticeKit> {
  const plan = randomHex(16);
  const round = unlockRound(nowMs / 1000 + PRACTICE_DELAY_SECONDS);
  const iso = (sec: number): string => new Date(sec * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z');
  const payload = {
    vault_seed: '',
    birthday_height: 0,
    network: 'main',
    plan,
    epoch: 1,
    message: 'Practice kit: no funds.',
    practice: true,
  };
  const sealed = await sealPayload(payload, [heirCode], round);
  const kit = {
    format: 'zandglass-kit',
    version: 1,
    kind: 'legacy',
    network: 'main',
    created: iso(Math.floor(nowMs / 1000)),
    plan,
    epoch: 1,
    unlock: { drand_chain: QUICKNET_INFO.hash, round, time: iso(roundTime(round)) },
    sealed,
  };
  return { text: JSON.stringify(kit), fileName: `practice-${plan.slice(0, 8)}.zandglass`, round };
}
