/**
 * Heir keys. A heir key is 24 BIP 39 words. The words become an X25519 secret
 * (HKDF-SHA256 over the BIP 39 seed, domain "sandglass-heir-v1") that is used
 * as an age identity. The heir code is the matching age recipient (`age1...`).
 * Nothing here touches the network or storage.
 */
import { bech32 } from '@scure/base';
import { hkdf } from '@noble/hashes/hkdf.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { generateMnemonic, mnemonicToSeedSync, validateMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { identityToRecipient } from 'age-encryption';

const DOMAIN = 'sandglass-heir-v1';

export class InvalidPhraseError extends Error {
  constructor(message = 'invalid heir phrase') {
    super(message);
    this.name = 'InvalidPhraseError';
  }
}

/** Lower-case, trim and collapse whitespace so pasted phrases compare cleanly. */
export function normalizePhrase(input: string): string {
  return input.toLowerCase().split(/\s+/).filter(Boolean).join(' ');
}

/** A fresh 24-word phrase from the browser's secure random generator. */
export function generatePhrase(): string {
  return generateMnemonic(wordlist, 256);
}

export function isValidPhrase(input: string): boolean {
  const phrase = normalizePhrase(input);
  return phrase.split(' ').length === 24 && validateMnemonic(phrase, wordlist);
}

/** phrase -> BIP 39 seed -> HKDF-SHA256("sandglass-heir-v1") -> AGE-SECRET-KEY-1... */
export function identityFromPhrase(input: string): string {
  const phrase = normalizePhrase(input);
  if (!isValidPhrase(phrase)) throw new InvalidPhraseError();
  const seed = mnemonicToSeedSync(phrase, '');
  const secret = hkdf(sha256, seed, undefined, new TextEncoder().encode(DOMAIN), 32);
  return bech32.encode('age-secret-key-', bech32.toWords(secret), 1023).toUpperCase();
}

/** The public heir code (`age1...`) that the owner needs. */
export async function heirCodeFromPhrase(input: string): Promise<string> {
  return identityToRecipient(identityFromPhrase(input));
}

/** `count` distinct 1-based word positions in ascending order. */
export function pickPositions(count: number, total: number): number[] {
  const picked = new Set<number>();
  const buf = new Uint32Array(1);
  while (picked.size < count) {
    crypto.getRandomValues(buf);
    picked.add(((buf[0] ?? 0) % total) + 1);
  }
  return [...picked].sort((a, b) => a - b);
}
