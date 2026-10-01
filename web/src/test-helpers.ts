/** Shared helpers for tests: the committed vectors and an offline drand client. */
import { readFileSync } from 'node:fs';
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
