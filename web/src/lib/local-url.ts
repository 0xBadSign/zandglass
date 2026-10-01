/**
 * Query and fragment overrides that point a page at another keeper (`?api=`,
 * `&k=`) exist for development and testing only. Anything but a local
 * http origin is ignored, so a crafted link cannot make a page send tokens or
 * kits to a server of the attacker's choosing.
 */

const LOCAL_HOSTS = new Set(['127.0.0.1', 'localhost']);

/** The origin of `raw` when it is `http://127.0.0.1:*` or `http://localhost:*`, else null. */
export function localKeeperOrigin(raw: string): string | null {
  try {
    const u = new URL(raw);
    if (u.protocol === 'http:' && LOCAL_HOSTS.has(u.hostname)) return u.origin;
  } catch {
    // not a URL
  }
  return null;
}

/** Same as {@link localKeeperOrigin}, with a console warning when the value is refused. */
export function acceptLocalKeeper(raw: string, what: string): string | null {
  const origin = localKeeperOrigin(raw);
  if (origin === null) {
    console.warn(`Ignoring ${what}: only http://127.0.0.1 and http://localhost are accepted.`);
  }
  return origin;
}
