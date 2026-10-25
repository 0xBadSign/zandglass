import { describe, expect, it } from 'vitest';
import { dateToUntil, untilEpoch } from './lock';

describe('savings lock helpers', () => {
  it('a picked date becomes the start of that day, as UTC RFC 3339', () => {
    const v = dateToUntil('2027-06-01');
    expect(v).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
    // It is local midnight, whatever the zone of the machine running this.
    expect(new Date(v ?? '').getTime()).toBe(new Date(2027, 5, 1).getTime());
  });

  it('rejects things that are not real dates', () => {
    expect(dateToUntil('')).toBeNull();
    expect(dateToUntil('2027-02-30')).toBeNull();
    expect(dateToUntil('2027-13-01')).toBeNull();
    expect(dateToUntil('tomorrow')).toBeNull();
  });

  it('turns durations and RFC 3339 times into a unix time', () => {
    expect(untilEpoch('90d', 1_000)).toBe(1_000 + 90 * 86_400);
    expect(untilEpoch('10m', 1_000)).toBe(1_600);
    expect(untilEpoch('1w', 0)).toBe(604_800);
    expect(untilEpoch('2026-12-01T09:00:00Z', 5)).toBe(1_796_115_600);
    expect(untilEpoch('soon', 0)).toBeNull();
    expect(untilEpoch('0d', 0)).toBeNull();
    expect(untilEpoch('2026-12-01', 0)).toBeNull();
  });
});
