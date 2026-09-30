import { describe, expect, it } from 'vitest';
import { QUICKNET, roundAt, roundTime, unlockRound } from './drand';

describe('drand quicknet round math', () => {
  it('starts at round 1 at genesis', () => {
    expect(roundAt(QUICKNET.genesisTime)).toBe(1);
    expect(roundTime(1)).toBe(QUICKNET.genesisTime);
  });

  it('unlocks no earlier than the requested time', () => {
    for (const t of [QUICKNET.genesisTime + 1, 1790000000, 1790000002]) {
      expect(roundTime(unlockRound(t))).toBeGreaterThanOrEqual(t);
      expect(roundTime(unlockRound(t)) - t).toBeLessThan(QUICKNET.period + 1);
    }
  });
});
