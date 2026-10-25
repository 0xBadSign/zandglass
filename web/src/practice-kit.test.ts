/** The heir fire drill: a practice kit sealed in the browser opens like a real kit. */
import { describe, expect, it } from 'vitest';
import { roundTime } from './drand';
import { generatePhrase, heirCodeFromPhrase } from './lib/heir';
import { WrongHeirKeyError, openKit, parseKit, sealPracticeKit } from './lib/kit';
import { read, recordedBeacon, recordedClient } from './test-helpers';

const PHRASE = read('TEST_ONLY_phrase.txt').trim();
// Seal exactly 60 s before the recorded round so the recorded beacon opens it.
const NOW_MS = (roundTime(recordedBeacon().round) - 60) * 1000;

describe('practice kit', () => {
  it('parses, opens with the heir phrase, and holds no seed', async () => {
    const heirCode = await heirCodeFromPhrase(PHRASE);
    const practice = await sealPracticeKit(heirCode, NOW_MS);
    expect(practice.fileName).toMatch(/^practice-[0-9a-f]{8}\.zandglass$/);

    const kit = parseKit(practice.text);
    expect(kit.round).toBe(recordedBeacon().round);
    expect(kit.meta?.kind).toBe('legacy');
    expect(kit.meta?.network).toBe('main');
    expect(kit.meta?.plan).toMatch(/^[0-9a-f]{32}$/);

    const opened = await openKit(kit, PHRASE, recordedClient());
    expect(opened.kind).toBe('json');
    if (opened.kind !== 'json') return;
    expect(opened.payload.practice).toBe(true);
    expect(opened.payload.vault_seed).toBe('');
    expect(opened.payload.message).toBe('Practice kit: no funds.');
  });

  it('does not open with another phrase', async () => {
    const heirCode = await heirCodeFromPhrase(PHRASE);
    const practice = await sealPracticeKit(heirCode, NOW_MS);
    await expect(openKit(parseKit(practice.text), generatePhrase(), recordedClient())).rejects.toBeInstanceOf(
      WrongHeirKeyError,
    );
  });

  it('uses a fresh plan id each time', async () => {
    const heirCode = await heirCodeFromPhrase(PHRASE);
    const a = parseKit((await sealPracticeKit(heirCode, NOW_MS)).text);
    const b = parseKit((await sealPracticeKit(heirCode, NOW_MS)).text);
    expect(a.meta?.plan).not.toBe(b.meta?.plan);
  });
});
