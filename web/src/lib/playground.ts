/**
 * Playground client logic (docs/PLAYGROUND_API.md). Pure helpers plus a small
 * fetch wrapper. Nothing here touches the DOM or stores anything.
 */

export type SessionState =
  | 'funding'
  | 'sealed'
  | 'flipping'
  | 'unlocked'
  | 'settling'
  | 'returned'
  | 'sent'
  | 'kept'
  | 'failed';

export interface PlaygroundTx {
  kind: string;
  txid: string;
  explorer: string;
}

export interface SessionSnapshot {
  id: string;
  state: SessionState;
  epoch: number;
  unlock_time: string | null;
  balance_zat: number;
  confirmed: boolean;
  txs: PlaygroundTx[];
  claim_link: string | null;
  message: string;
  error: string | null;
}

export interface CreatedSession {
  id: string;
  token: string;
  claim_link: string;
  network: string;
  amount_zat: number;
  minutes: number;
}

export interface PlaygroundStatus {
  enabled: boolean;
  network: string;
  amount_zat: number;
  active: number;
  max_active: number;
  today: number;
  max_per_day: number;
  float_ready: boolean;
}

export const MINUTE_CHOICES = [3, 5, 10] as const;
export const TERMINAL_STATES: readonly SessionState[] = ['returned', 'sent', 'kept', 'failed'];

/* ------------------------------------------------------------ errors */

export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  constructor(code: string, status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}

const MESSAGES: Record<string, string> = {
  judge_code_invalid: 'That judge code is not accepted. Check it and try again.',
  playground_closed: 'The playground is closed right now. Please come back later.',
  playground_busy: 'The playground is busy. Too many people are trying it at the moment. Wait a few minutes and try again.',
  daily_cap_reached: 'The playground has reached its limit for today. Please try again tomorrow.',
  session_not_found: 'This session was not found. Sessions end by themselves after a while. Start a new one.',
  invalid_state: 'That action is not available right now. Wait for the next step and try again.',
  unauthorized: 'This session does not accept that request. Start a new session.',
  kit_invalid: 'The kit could not be read. Start a new session.',
  kit_version_unsupported: 'The kit uses a version this page does not support.',
  not_yet_unlocked: 'The unlock time has not passed yet. Wait for the countdown to finish.',
  beacon_unavailable: 'The time key is not available yet. Try again in a few seconds.',
  wrong_heir_key: 'These words do not open this kit.',
  decrypt_failed: 'The kit could not be opened.',
  vault_empty: 'This vault has already been moved or emptied.',
  network_mismatch: 'That address is for a different Zcash network than this demo. Check the network label.',
  amount_over_cap: 'That amount is over the demo limit.',
  insufficient_funds: 'The demo wallet cannot fund a new vault right now. Please try again later.',
  sync_required: 'The server is still syncing. Try again in a minute.',
  mailbox_not_found: 'The kit mailbox was not found. Start a new session.',
  bad_request: 'The server did not accept that request. Check what you entered and try again.',
};

/** A plain-English message for a contract error code. */
export function friendlyError(code: string, status = 0): string {
  const known = MESSAGES[code];
  if (known) return known;
  if (status === 429) return MESSAGES['playground_busy'] ?? '';
  if (status === 404) return MESSAGES['session_not_found'] ?? '';
  if (status === 401 || status === 403) return MESSAGES['unauthorized'] ?? '';
  if (status >= 500) return 'The playground server had a problem. Wait a moment and try again.';
  return 'Something went wrong. Try again in a moment.';
}

/** True for errors worth retrying by polling again (not the visitor's fault). */
export function isTransient(e: unknown): boolean {
  if (!(e instanceof ApiError)) return true;
  return e.code === 'network' || e.status >= 500;
}

/* ------------------------------------------------------------ api */

/** API base: same origin, or `?api=<url>` for local testing (http/https only). */
export function apiBase(search: string, origin: string): string {
  const raw = new URLSearchParams(search).get('api');
  if (raw) {
    try {
      const u = new URL(raw);
      if (u.protocol === 'https:' || u.protocol === 'http:') {
        return `${u.origin}${u.pathname.replace(/\/+$/, '')}`;
      }
    } catch {
      // fall through to the default
    }
  }
  return origin;
}

/** Poll interval in ms: 3000 by default, `?poll=<ms>` (200..10000) for tests. */
export function pollMs(search: string): number {
  const n = Number(new URLSearchParams(search).get('poll'));
  return Number.isInteger(n) && n >= 200 && n <= 10_000 ? n : 3000;
}

export interface RequestOptions {
  method?: 'GET' | 'POST';
  token?: string;
  body?: unknown;
}

/** One JSON request to the keeper. Throws ApiError (code "network" if unreachable). */
export async function request<T>(
  base: string,
  path: string,
  opts: RequestOptions = {},
  fetchFn: typeof fetch = (...a) => fetch(...a),
): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' };
  if (opts.token) headers['Authorization'] = `Bearer ${opts.token}`;
  const init: RequestInit = {
    method: opts.method ?? 'GET',
    headers,
    cache: 'no-store',
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  };
  if (opts.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(opts.body);
  }
  let res: Response;
  try {
    res = await fetchFn(`${base}${path}`, init);
  } catch {
    throw new ApiError('network', 0, 'Could not reach the playground server. Check your internet connection.');
  }
  let json: unknown = null;
  try {
    json = await res.json();
  } catch {
    json = null;
  }
  if (!res.ok) {
    const code =
      typeof json === 'object' && json !== null && typeof (json as Record<string, unknown>)['error'] === 'string'
        ? String((json as Record<string, unknown>)['error'])
        : `http_${res.status}`;
    throw new ApiError(code, res.status, friendlyError(code, res.status));
  }
  if (typeof json !== 'object' || json === null) {
    throw new ApiError('bad_response', res.status, 'The server sent an answer this page cannot read.');
  }
  return json as T;
}

/* ------------------------------------------------------------ claim link */

/** The mailbox id and read token inside a claim link's `#m=..&r=..` fragment. */
export function parseClaimLink(link: string): { id: string; token: string } | null {
  const i = link.indexOf('#');
  if (i < 0) return null;
  const params = new URLSearchParams(link.slice(i + 1));
  const id = params.get('m');
  const token = params.get('r');
  if (!id || !token) return null;
  return { id, token };
}

/* ------------------------------------------------------------ formatting */

export function formatCountdown(totalSeconds: number): string {
  const s = Math.max(0, Math.ceil(totalSeconds));
  const d = Math.floor(s / 86400);
  const hh = Math.floor((s % 86400) / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const p = (n: number) => String(n).padStart(2, '0');
  if (d > 0) return `${d}d ${p(hh)}h ${p(mm)}m ${p(ss)}s`;
  if (hh > 0) return `${hh}h ${p(mm)}m ${p(ss)}s`;
  return `${p(mm)}m ${p(ss)}s`;
}

/** 20000 zat -> "0.0002 ZEC". */
export function formatZec(zat: number, unit = 'ZEC'): string {
  const whole = Math.floor(zat / 1e8);
  const frac = String(Math.round(zat % 1e8)).padStart(8, '0').replace(/0+$/, '');
  return `${whole}${frac ? `.${frac}` : ''} ${unit}`;
}

export function networkLabel(network: string): { text: string; test: boolean } {
  return network === 'test'
    ? { text: 'Testnet (test coins)', test: true }
    : { text: 'Mainnet (real ZEC)', test: false };
}

/** Light client-side check; the server is the judge of the address. */
export function looksLikeUnifiedAddress(addr: string, network: string): boolean {
  const a = addr.trim();
  const prefix = network === 'test' ? 'utest1' : 'u1';
  return a.startsWith(prefix) && a.length >= 40 && /^[a-z0-9]+$/.test(a);
}

/** Only https links are rendered as links. */
export function safeHref(url: string): string | null {
  try {
    const u = new URL(url);
    return u.protocol === 'https:' ? u.href : null;
  } catch {
    return null;
  }
}

/** Fraction (0..1) of the timer that has passed. */
export function elapsedFraction(unlockMs: number, minutes: number, nowMs: number): number {
  const total = minutes * 60_000;
  if (total <= 0) return 1;
  return Math.min(1, Math.max(0, (nowMs - (unlockMs - total)) / total));
}

/* ------------------------------------------------------------ timeline */

export type StepStatus = 'done' | 'active' | 'todo' | 'skipped' | 'failed';

export interface TimelineStep {
  id: 'funded' | 'sealed' | 'flip' | 'sand' | 'open';
  label: string;
  status: StepStatus;
  detail: string;
  txs: PlaygroundTx[];
}

const RANK: Record<SessionState, number> = {
  funding: 0,
  sealed: 1,
  flipping: 1,
  unlocked: 3,
  settling: 4,
  returned: 5,
  sent: 5,
  kept: 5,
  failed: -1,
};

/** How far along a state is (failed has no rank of its own). */
export function stateRank(state: SessionState): number {
  return RANK[state];
}

/** The five stages shown to the visitor: Funded, Sealed, Flip?, Sand runs out, Open. */
export function buildTimeline(s: SessionSnapshot | null, lastRank = 0): TimelineStep[] {
  const state: SessionState = s?.state ?? 'funding';
  const failed = state === 'failed';
  const rank = failed ? lastRank : RANK[state];
  const txs = s?.txs ?? [];
  const fund = txs.filter((t) => t.kind === 'fund');
  const flips = txs.filter((t) => t.kind === 'flip');
  const settle = txs.filter((t) => t.kind === 'return' || t.kind === 'send');

  const stage = (at: number): StepStatus => {
    if (failed && rank === at) return 'failed';
    if (rank > at) return 'done';
    if (rank === at) return 'active';
    return 'todo';
  };
  const flipStatus: StepStatus =
    state === 'flipping' ? 'active' : flips.length > 0 ? 'done' : rank >= 3 ? 'skipped' : 'todo';
  const openStatus: StepStatus =
    failed && rank >= 3 ? 'failed' : rank >= 5 ? 'done' : rank >= 3 ? 'active' : 'todo';

  return [
    {
      id: 'funded',
      label: 'Funded',
      status: stage(0),
      detail: fund.length ? 'The vault received the demo coins.' : 'The demo wallet funds a new vault.',
      txs: fund,
    },
    {
      id: 'sealed',
      label: 'Sealed',
      status: rank >= 1 ? 'done' : 'todo',
      detail: 'The kit is locked to the unlock time and to your heir code.',
      txs: [],
    },
    {
      id: 'flip',
      label: 'Flip?',
      status: flipStatus,
      detail:
        flips.length > 0
          ? `Flipped ${flips.length} time${flips.length === 1 ? '' : 's'}. Each flip moved the funds to a new vault.`
          : 'Optional. A flip moves the funds to a new vault and resets the timer.',
      txs: flips,
    },
    {
      id: 'sand',
      label: 'Sand runs out',
      status: rank >= 3 ? 'done' : rank >= 1 ? 'active' : 'todo',
      detail: 'When the unlock time passes, nobody can stop the kit from opening.',
      txs: [],
    },
    {
      id: 'open',
      label: 'Open',
      status: openStatus,
      detail: 'You open the kit here, in your browser, and decide what happens to the coins.',
      txs: settle,
    },
  ];
}

/* ------------------------------------------------------------ live vault */

export interface LiveVault {
  network: string;
  unlockMs: number;
  claimTx: string | null;
}

/** Parse `live-vault.json`; null when it is missing or malformed. */
export function parseLiveVault(json: unknown): LiveVault | null {
  if (typeof json !== 'object' || json === null) return null;
  const j = json as Record<string, unknown>;
  const network = j['network'];
  const time = j['unlock_time'];
  if (typeof network !== 'string' || typeof time !== 'string') return null;
  const ms = Date.parse(time);
  if (!Number.isFinite(ms)) return null;
  const tx = j['claim_tx'];
  return { network, unlockMs: ms, claimTx: typeof tx === 'string' && /^[0-9a-f]{64}$/.test(tx) ? tx : null };
}
