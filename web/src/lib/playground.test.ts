import { describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  apiBase,
  buildTimeline,
  elapsedFraction,
  formatCountdown,
  formatZec,
  friendlyError,
  looksLikeUnifiedAddress,
  parseClaimLink,
  parseLiveVault,
  pollMs,
  request,
  safeHref,
  type SessionSnapshot,
} from './playground';

const snap = (over: Partial<SessionSnapshot>): SessionSnapshot => ({
  id: 'a',
  state: 'funding',
  epoch: 1,
  unlock_time: null,
  balance_zat: 0,
  confirmed: false,
  txs: [],
  claim_link: null,
  message: '',
  error: null,
  ...over,
});

describe('errors', () => {
  it('has a plain message for every contract code', () => {
    for (const c of [
      'judge_code_invalid',
      'playground_closed',
      'playground_busy',
      'daily_cap_reached',
      'session_not_found',
      'invalid_state',
      'unauthorized',
      'kit_invalid',
      'not_yet_unlocked',
      'beacon_unavailable',
      'wrong_heir_key',
      'vault_empty',
      'network_mismatch',
      'amount_over_cap',
      'insufficient_funds',
      'sync_required',
      'mailbox_not_found',
    ]) {
      expect(friendlyError(c)).toMatch(/^[A-Z].*[.]$/);
    }
  });
  it('falls back on the HTTP status', () => {
    expect(friendlyError('weird', 429)).toContain('busy');
    expect(friendlyError('weird', 500)).toContain('server');
    expect(friendlyError('weird', 0)).toContain('Something went wrong');
  });
});

async function fail(p: Promise<unknown>): Promise<ApiError> {
  try {
    await p;
  } catch (e) {
    return e as ApiError;
  }
  throw new Error('expected a failure');
}

describe('request', () => {
  const reply = (status: number, body: unknown) =>
    (async () => new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;
  it('returns JSON and sends the bearer token', async () => {
    let seen: RequestInit | undefined;
    const f = (async (_u: string, init?: RequestInit) => {
      seen = init;
      return new Response('{"ok":1}', { status: 200 });
    }) as unknown as typeof fetch;
    expect(await request('http://x', '/p', { token: 't', body: { a: 1 }, method: 'POST' }, f)).toEqual({ ok: 1 });
    expect((seen?.headers as Record<string, string>)['Authorization']).toBe('Bearer t');
    expect(seen?.credentials).toBe('omit');
  });
  it('maps error bodies to ApiError', async () => {
    const e = await fail(request('http://x', '/p', {}, reply(403, { error: 'judge_code_invalid', message: 'x' })));
    expect(e).toBeInstanceOf(ApiError);
    expect(e.code).toBe('judge_code_invalid');
    expect(e.message).toContain('judge code');
  });
  it('maps unreachable servers to code network', async () => {
    const f = (async () => {
      throw new TypeError('fail');
    }) as unknown as typeof fetch;
    const e = await fail(request('http://x', '/p', {}, f));
    expect(e.code).toBe('network');
  });
  it('rejects non-JSON success bodies', async () => {
    const f = (async () => new Response('<html>', { status: 200 })) as unknown as typeof fetch;
    const e = await fail(request('http://x', '/p', {}, f));
    expect(e.code).toBe('bad_response');
  });
});

describe('api base and poll', () => {
  it('defaults to the page origin and accepts ?api for local http only', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    expect(apiBase('', 'https://a.example')).toBe('https://a.example');
    expect(apiBase('?api=http://localhost:8787/', 'https://a.example')).toBe('http://localhost:8787');
    expect(apiBase('?api=http://127.0.0.1:8787', 'https://a.example')).toBe('http://127.0.0.1:8787');
    expect(warn).not.toHaveBeenCalled();
    for (const bad of [
      'javascript:alert(1)',
      'nonsense',
      'https://evil.example',
      'http://evil.example',
      'http://localhost.evil.example',
      'http://127.0.0.1.evil.example',
      'https://localhost:8787',
    ]) {
      expect(apiBase(`?api=${bad}`, 'https://a.example')).toBe('https://a.example');
    }
    expect(warn).toHaveBeenCalledTimes(7);
    warn.mockRestore();
  });
  it('polls every 3 s unless a sane override is given', () => {
    expect(pollMs('')).toBe(3000);
    expect(pollMs('?poll=300')).toBe(300);
    expect(pollMs('?poll=5')).toBe(3000);
    expect(pollMs('?poll=abc')).toBe(3000);
  });
});

describe('claim link', () => {
  it('reads the mailbox id and token', () => {
    expect(parseClaimLink('https://h.example/claim.html#m=abc&r=def')).toEqual({ id: 'abc', token: 'def' });
    expect(parseClaimLink('https://h.example/claim.html#m=abc')).toBeNull();
    expect(parseClaimLink('https://h.example/claim.html')).toBeNull();
  });
});

describe('formatting', () => {
  it('formats countdowns', () => {
    expect(formatCountdown(65)).toBe('01m 05s');
    expect(formatCountdown(3725)).toBe('1h 02m 05s');
    expect(formatCountdown(-4)).toBe('00m 00s');
  });
  it('formats zatoshi', () => {
    expect(formatZec(20000)).toBe('0.0002 ZEC');
    expect(formatZec(100000000)).toBe('1 ZEC');
    expect(formatZec(150000000, 'TAZ')).toBe('1.5 TAZ');
  });
  it('checks unified addresses loosely by network', () => {
    const body = 'q'.repeat(60);
    expect(looksLikeUnifiedAddress(`u1${body}`, 'main')).toBe(true);
    expect(looksLikeUnifiedAddress(`utest1${body}`, 'main')).toBe(false);
    expect(looksLikeUnifiedAddress(`utest1${body}`, 'test')).toBe(true);
    expect(looksLikeUnifiedAddress('u1short', 'main')).toBe(false);
    expect(looksLikeUnifiedAddress(`u1${body} `, 'main')).toBe(true);
  });
  it('only links https explorers', () => {
    expect(safeHref('https://zcashexplorer.app/tx/ab')).toContain('https://');
    expect(safeHref('javascript:alert(1)')).toBeNull();
    expect(safeHref('http://x.example')).toBeNull();
  });
  it('computes how much sand has fallen', () => {
    expect(elapsedFraction(1_000_000 + 300_000, 5, 1_000_000)).toBe(0);
    expect(elapsedFraction(1_000_000 + 150_000, 5, 1_000_000)).toBeCloseTo(0.5);
    expect(elapsedFraction(1_000_000, 5, 5_000_000)).toBe(1);
  });
});

describe('timeline', () => {
  const ids = (s: ReturnType<typeof buildTimeline>) => s.map((x) => x.status);
  it('starts at funding', () => {
    expect(ids(buildTimeline(null))).toEqual(['active', 'todo', 'todo', 'todo', 'todo']);
  });
  it('counts down once sealed', () => {
    expect(ids(buildTimeline(snap({ state: 'sealed' })))).toEqual(['done', 'done', 'todo', 'active', 'todo']);
  });
  it('shows a flip in progress and a finished flip', () => {
    expect(buildTimeline(snap({ state: 'flipping' }))[2]?.status).toBe('active');
    const t = buildTimeline(snap({ state: 'sealed', txs: [{ kind: 'flip', txid: 'x', explorer: 'https://e/x' }] }));
    expect(t[2]?.status).toBe('done');
    expect(t[2]?.txs).toHaveLength(1);
  });
  it('skips the flip when none happened, then opens', () => {
    expect(ids(buildTimeline(snap({ state: 'unlocked' })))).toEqual(['done', 'done', 'skipped', 'done', 'active']);
    expect(ids(buildTimeline(snap({ state: 'returned' })))).toEqual(['done', 'done', 'skipped', 'done', 'done']);
  });
  it('marks the stage that failed', () => {
    expect(ids(buildTimeline(snap({ state: 'failed' }), 0))[0]).toBe('failed');
    expect(ids(buildTimeline(snap({ state: 'failed' }), 3))[4]).toBe('failed');
  });
});

describe('live vault', () => {
  it('parses the file and tolerates junk', () => {
    const tx = 'a'.repeat(64);
    expect(parseLiveVault({ network: 'main', unlock_time: '2026-11-01T00:00:00Z', claim_tx: null })).toEqual({
      network: 'main',
      unlockMs: Date.parse('2026-11-01T00:00:00Z'),
      claimTx: null,
    });
    expect(parseLiveVault({ network: 'main', unlock_time: '2026-11-01T00:00:00Z', claim_tx: tx })?.claimTx).toBe(tx);
    expect(parseLiveVault({ network: 'main', unlock_time: 'nope' })).toBeNull();
    expect(parseLiveVault('<html>')).toBeNull();
    expect(parseLiveVault(null)).toBeNull();
  });
});
