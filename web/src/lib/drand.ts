/** drand quicknet access for the browser: relay fallback, no third parties. */
import type { ChainClient, ChainInfo } from 'drand-client';
import { QUICKNET } from '../drand';

export const RELAYS = [
  'https://api.drand.sh',
  'https://api2.drand.sh',
  'https://drand.cloudflare.com',
] as const;

export const QUICKNET_INFO: ChainInfo = {
  public_key:
    '83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a',
  period: 3,
  genesis_time: 1692803367,
  hash: QUICKNET.chainHash,
  groupHash: 'f477d5c89f21a17c863a7f937c6a6d15859414d2be09cd448d4279af331c5d3e',
  schemeID: 'bls-unchained-g1-rfc9380',
  metadata: { beaconID: 'quicknet' },
};

/** No relay could be reached (offline, blocked, or all relays down). */
export class NetworkError extends Error {
  constructor(message = 'network error') {
    super(message);
    this.name = 'NetworkError';
  }
}

/** The relays answered, but the beacon for this round is not published yet. */
export class BeaconNotReadyError extends Error {
  constructor(message = 'beacon not published yet') {
    super(message);
    this.name = 'BeaconNotReadyError';
  }
}

async function fetchJson(url: string, timeoutMs: number, fetchFn: typeof fetch): Promise<unknown> {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetchFn(url, {
      signal: ctl.signal,
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
    });
    if (res.status === 404) throw new BeaconNotReadyError();
    if (!res.ok) throw new NetworkError(`relay answered ${res.status}`);
    return await res.json();
  } catch (e) {
    if (e instanceof BeaconNotReadyError || e instanceof NetworkError) throw e;
    throw new NetworkError(e instanceof Error ? e.message : 'request failed');
  } finally {
    clearTimeout(timer);
  }
}

/**
 * A quicknet chain client that asks every relay at once and takes the first
 * good answer, so one slow or unreachable relay does not stall the claim. The
 * chain info is pinned, so only `/public/<round>` is fetched. The beacon
 * signature is the decryption key, so a forged beacon simply fails to open the kit.
 */
export function relayClient(
  relays: readonly string[] = RELAYS,
  fetchFn: typeof fetch = (...a) => fetch(...a),
  timeoutMs = 8000,
): ChainClient {
  const cache = new Map<number, unknown>();
  async function get(round: number): Promise<never> {
    if (cache.has(round)) return cache.get(round) as never;
    let notReady = false;
    try {
      const beacon = await Promise.any(
        relays.map((relay) =>
          fetchJson(`${relay}/${QUICKNET.chainHash}/public/${round}`, timeoutMs, fetchFn).catch((e: unknown) => {
            if (e instanceof BeaconNotReadyError) notReady = true;
            throw e;
          }),
        ),
      );
      cache.set(round, beacon);
      return beacon as never;
    } catch {
      throw notReady ? new BeaconNotReadyError() : new NetworkError();
    }
  }
  return {
    options: { disableBeaconVerification: true, noCache: true },
    latest: async () => {
      throw new Error('latest is not supported');
    },
    get,
    chain: () => ({ baseUrl: relays[0] ?? '', info: async () => QUICKNET_INFO }),
  };
}
