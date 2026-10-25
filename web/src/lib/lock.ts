/*
 * Savings-lock helpers for the owner panel: turn the form's "until" choices
 * into what the CLI and `zandglass ui` accept (an RFC 3339 UTC time such as
 * 2027-06-01T09:00:00Z, or a duration such as 90d), and preview the result.
 */

const UNITS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86400, w: 604800 };

/**
 * A date chosen in a date picker (YYYY-MM-DD) becomes the start of that day in
 * the owner's own time zone, as an RFC 3339 UTC time. Null when it is not a
 * real calendar date.
 */
export function dateToUntil(date: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date.trim());
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const at = new Date(y, mo - 1, d);
  if (at.getFullYear() !== y || at.getMonth() !== mo - 1 || at.getDate() !== d) return null;
  return at.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

/**
 * The unix time (seconds) an "until" value stands for: an RFC 3339 UTC time,
 * or a duration counted from `nowSec`. Null when it is neither.
 */
export function untilEpoch(value: string, nowSec: number): number | null {
  const v = value.trim();
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(v)) {
    const t = Date.parse(v);
    return Number.isNaN(t) ? null : Math.floor(t / 1000);
  }
  const m = /^(\d+)([smhdw])$/.exec(v);
  if (!m) return null;
  const secs = Number(m[1]) * (UNITS[m[2] ?? ''] ?? 0);
  return secs > 0 ? Math.floor(nowSec) + secs : null;
}
