/** drand quicknet parameters (verified against the network's /info in T0.4). */
export const QUICKNET = {
  chainHash: '52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971',
  genesisTime: 1692803367,
  period: 3,
} as const;

/** The round being published at unix time `t` (seconds). */
export function roundAt(t: number): number {
  return Math.floor((t - QUICKNET.genesisTime) / QUICKNET.period) + 1;
}

/** The first round whose beacon is published at or after unix time `t`. */
export function unlockRound(t: number): number {
  return Math.ceil((t - QUICKNET.genesisTime) / QUICKNET.period) + 1;
}

/** Unix time (seconds) at which `round` is published. */
export function roundTime(round: number): number {
  return QUICKNET.genesisTime + (round - 1) * QUICKNET.period;
}
