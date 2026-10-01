/**
 * Kits (PLAN 5.4): public JSON metadata around a `sealed` tlock armour string.
 * sealed = tlock_R( age_to(heir_codes, payload) ). Everything here runs in the
 * browser; the phrase and the decrypted payload never leave it.
 */
import { Decrypter } from 'age-encryption';
import type { ChainClient } from 'drand-client';
import { Buffer, timelockDecrypt } from 'tlock-js';
import { BeaconNotReadyError, NetworkError } from './drand';
import { identityFromPhrase } from './heir';
import { roundTime } from '../drand';

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
}

export interface Payload {
  vault_seed: string;
  birthday_height: number;
  network?: string;
  plan?: string;
  epoch?: number;
  message?: string;
  restore_hints?: string[];
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
  return parsePayload(text);
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
      return { kind: 'json', payload };
    }
  } catch {
    // not JSON: fall through and show the text as it is
  }
  return { kind: 'raw', text };
}
