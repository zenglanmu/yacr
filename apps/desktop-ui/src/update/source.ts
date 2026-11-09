// The one network read: fetch the landing manifest, validate it, never throw.
//
// The result is a discriminated union rather than an exception because every
// caller treats a failure as a state (`unavailable`), not as a crash. The read is
// bounded three ways: `AbortController` + `FETCH_TIMEOUT_MS`, the declared
// `Content-Length` when present, and the decoded UTF-8 length. `cache: 'no-store'`
// keeps a stale proxy copy from being mistaken for the current release.

import { FETCH_TIMEOUT_MS, MAX_MANIFEST_BYTES, RELEASE_MANIFEST_URL } from './config.ts';
import { parseManifest, utf8Length } from './manifest.ts';

/** A successful read, or the stable reason it failed. */
export type FetchLatestResult =
  | { readonly ok: true; readonly release: string }
  | { readonly ok: false; readonly reason: string };

/** What a caller may inject (the network, the clock, the budget, the URL). */
export interface FetchLatestDeps {
  /** The `fetch` to use; defaults to the global one. */
  readonly fetch?: typeof globalThis.fetch;
  /** The clock, kept injectable for symmetry with the store; unused by design. */
  readonly now?: () => number;
  /** The abort budget in ms; defaults to [`FETCH_TIMEOUT_MS`]. */
  readonly timeout?: number;
  /** The manifest URL; defaults to [`RELEASE_MANIFEST_URL`]. */
  readonly url?: string;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function timeoutOf(value: number | undefined): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : FETCH_TIMEOUT_MS;
}

/** The declared body length, when the header is present and a usable number. */
function declaredLength(response: Response): number | null {
  try {
    const raw = response.headers?.get?.('content-length');
    if (typeof raw !== 'string' || raw.trim() === '') return null;
    const value = Number(raw);
    return Number.isFinite(value) && value >= 0 ? value : null;
  } catch {
    return null;
  }
}

/**
 * Fetch and validate the manifest.
 *
 * Non-2xx, an oversized body (declared or decoded), a timeout, a transport throw
 * and a schema failure all resolve to `{ok: false, reason}`. There is no path
 * that rejects.
 */
export async function fetchLatestRelease(deps: FetchLatestDeps = {}): Promise<FetchLatestResult> {
  const fetchImpl = deps.fetch
    ?? (typeof globalThis.fetch === 'function' ? globalThis.fetch : undefined);
  if (typeof fetchImpl !== 'function') {
    return { ok: false, reason: 'update.fetch_unavailable' };
  }
  const timeout = timeoutOf(deps.timeout);
  const controller = new AbortController();
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeout);
  try {
    const response = await fetchImpl(deps.url ?? RELEASE_MANIFEST_URL, {
      cache: 'no-store',
      signal: controller.signal,
    });
    if (response === null || typeof response !== 'object') {
      return { ok: false, reason: 'update.invalid_response' };
    }
    const status = typeof response.status === 'number' ? response.status : 0;
    if (response.ok !== true) {
      return { ok: false, reason: `update.http_status: ${status}` };
    }
    const declared = declaredLength(response);
    if (declared !== null && declared > MAX_MANIFEST_BYTES) {
      return { ok: false, reason: `update.manifest_too_large: ${declared} > ${MAX_MANIFEST_BYTES}` };
    }
    if (typeof response.text !== 'function') {
      return { ok: false, reason: 'update.invalid_body' };
    }
    const text = await response.text();
    if (typeof text !== 'string') return { ok: false, reason: 'update.invalid_body' };
    const decoded = utf8Length(text);
    if (decoded > MAX_MANIFEST_BYTES) {
      return { ok: false, reason: `update.manifest_too_large: ${decoded} > ${MAX_MANIFEST_BYTES}` };
    }
    const parsed = parseManifest(text);
    if (!parsed.ok) return { ok: false, reason: parsed.reason };
    return { ok: true, release: parsed.release };
  } catch (error) {
    if (timedOut) {
      return { ok: false, reason: `update.timeout: no answer within ${timeout}ms` };
    }
    return { ok: false, reason: `update.fetch_failed: ${errorMessage(error)}` };
  } finally {
    clearTimeout(timer);
  }
}
