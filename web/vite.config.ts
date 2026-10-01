import { resolve } from 'node:path';
import { defineConfig } from 'vitest/config';

// Served with a strict CSP in preview (and in production by the host): scripts
// and styles only from this origin, network only to the drand relays and the
// keeper (same origin by default).
const CSP = [
  "default-src 'none'",
  "script-src 'self'",
  "style-src 'self'",
  "font-src 'self'",
  "img-src 'self' data:",
  "media-src 'self'",
  "connect-src 'self' https://api.drand.sh https://api2.drand.sh https://drand.cloudflare.com",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'none'",
].join('; ');

const headers = {
  'Content-Security-Policy': CSP,
  'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff',
};

export default defineConfig({
  base: './',
  build: {
    target: 'es2022',
    // The CSP has `font-src 'self'`, which blocks the small fonts Vite would
    // otherwise inline as data: URLs. Emit them as files.
    assetsInlineLimit: 0,
    rollupOptions: {
      input: {
        index: resolve(__dirname, 'index.html'),
        heir: resolve(__dirname, 'heir.html'),
        claim: resolve(__dirname, 'claim.html'),
        playground: resolve(__dirname, 'playground.html'),
        owner: resolve(__dirname, 'owner.html'),
      },
    },
  },
  preview: { host: '127.0.0.1', port: 4173, strictPort: true, headers },
  test: { include: ['src/**/*.test.ts'] },
});
