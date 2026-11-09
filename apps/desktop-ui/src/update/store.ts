// The update-check store the chrome subscribes to.
//
// This module owns one session-scoped state machine and one session-scoped
// check. It is React-free except for the `useSyncExternalStore` hook, so the
// node tests import the same functions the notice component uses.
//
// # The state machine
//
//   idle -> checking -> { up-to-date | available | skipped | unsupported | unavailable }
//                         available -> idle      (dismissUpdate)
//                         available -> skipped   (skipUpdateVersion, persisted)
//
// Only `available` renders a notice. `unsupported` (the carrier cannot answer
// `cad_app_version`, e.g. the browser's wasm session) and `unavailable` (the host
// rejected, the network failed, the manifest was garbage) are *not* "up to date":
// a check that could not run must never claim the app is current.
//
// # One check per session
//
// `startUpdateCheck` is idempotent after its first call (`started`). A stale
// async result is still guarded by `generation`, so a test (or a future caller
// that resets the store) cannot let an older answer overwrite a newer one.
//
// # Storage
//
// The policy lives in `policy.ts`; this store only decides *when* to read it.
// `lastCheckAt` is written after a successful manifest read only, so a failed
// fetch is retried at the next startup instead of being suppressed for a day.

import { useSyncExternalStore } from 'react';
import { invoke as hostInvoke } from '../host/invoke.ts';
import type { Invoke } from '../host/invoke.ts';
import { fetchLatestRelease } from './source.ts';
import { formatVersion, isNewer, parseVersion } from './version.ts';
import { markChecked, markSkipped, shouldCheck, skippedVersion } from './policy.ts';
import type { UpdateStorage } from './policy.ts';

/** Everything the notice needs to know; only `available` is a visible notice. */
export type UpdateStatus =
  | { readonly kind: 'idle' }
  | { readonly kind: 'checking' }
  | { readonly kind: 'up-to-date'; readonly current: string }
  | { readonly kind: 'available'; readonly current: string; readonly latest: string }
  | { readonly kind: 'skipped'; readonly current: string; readonly latest: string }
  | { readonly kind: 'unsupported'; readonly reason: string }
  | { readonly kind: 'unavailable'; readonly reason: string };

/** What a caller (tests, and the session bootstrap) may inject. */
export interface UpdateDeps {
  /** The transport callable; defaults to the shared `host/invoke.ts` one. */
  readonly invoke?: Invoke;
  /** The clock; defaults to `Date.now`. */
  readonly now?: () => number;
  /** The manifest `fetch`; defaults to the global one. */
  readonly fetch?: typeof globalThis.fetch;
  /** The policy store; defaults to the page's `localStorage`. `null` disables it. */
  readonly storage?: UpdateStorage | null;
}

let status: UpdateStatus = { kind: 'idle' };
const listeners = new Set<() => void>();
let started = false;
let generation = 0;
/**
 * The storage the running check was started with, so `skipUpdateVersion` writes
 * to the same store the check read. `undefined` means "use the policy default".
 */
let policyStorage: UpdateStorage | null | undefined;

function emit(): void {
  for (const listener of [...listeners]) listener();
}

function setStatus(next: UpdateStatus): void {
  status = next;
  emit();
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A canonical version string, or `null` when it is not a usable release. */
function normalizeVersion(raw: string): string | null {
  const parsed = parseVersion(raw);
  return parsed === null ? null : formatVersion(parsed);
}

/** Whether `run` has been superseded by a newer check (or a reset). */
function isStale(run: number): boolean {
  return run !== generation;
}

/** Subscribe to status changes (the `useSyncExternalStore` contract). */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** The current status (a stable object identity until it changes). */
export function getUpdateStatus(): UpdateStatus {
  return status;
}

/** The React binding the notice uses. */
export function useUpdateStatus(): UpdateStatus {
  return useSyncExternalStore(subscribe, getUpdateStatus, getUpdateStatus);
}

async function runUpdateCheck(deps: UpdateDeps, run: number, force: boolean): Promise<void> {
  const invokeFn = deps.invoke ?? hostInvoke;
  const nowFn = deps.now ?? Date.now;
  const now = nowFn();
  setStatus({ kind: 'checking' });

  let rawCurrent: string;
  try {
    const answer = await invokeFn<string>('cad_app_version');
    rawCurrent = typeof answer === 'string' ? answer : String(answer ?? '');
  } catch (error) {
    if (isStale(run)) return;
    const reason = errorMessage(error);
    // A carrier that does not serve the command says so by name: no notice and
    // no "up to date" claim, just the explicit unsupported state.
    setStatus(reason.startsWith('unsupported-runtime:')
      ? { kind: 'unsupported', reason }
      : { kind: 'unavailable', reason });
    return;
  }
  if (isStale(run)) return;

  const current = normalizeVersion(rawCurrent);
  if (current === null) {
    setStatus({
      kind: 'unavailable',
      reason: `update.current_version_invalid: ${JSON.stringify(rawCurrent)}`,
    });
    return;
  }

  // Throttled: do not fetch. `idle` is the honest answer - the check did not
  // produce any comparison, and a re-evaluation only happens when a check runs.
  // A forced check (the About dialog's explicit action) bypasses this throttle.
  if (!force && !shouldCheck(now, deps.storage)) {
    setStatus({ kind: 'idle' });
    return;
  }

  const fetched = await fetchLatestRelease({ fetch: deps.fetch });
  if (isStale(run)) return;
  if (!fetched.ok) {
    setStatus({ kind: 'unavailable', reason: fetched.reason });
    return;
  }

  // A successful manifest read is recorded before interpreting it: the network
  // read happened, so the throttle applies even if the published version is
  // unusable. Only a *failed* fetch leaves the timestamp untouched.
  markChecked(now, deps.storage);

  const latest = normalizeVersion(fetched.release);
  if (latest === null) {
    setStatus({
      kind: 'unavailable',
      reason: `update.manifest_version_invalid: ${JSON.stringify(fetched.release)}`,
    });
    return;
  }

  if (!isNewer(latest, current)) {
    setStatus({ kind: 'up-to-date', current });
    return;
  }
  const skipped = skippedVersion(deps.storage);
  // An explicit check reports the release even if the user previously skipped
  // it; only the automatic startup check stays quiet. The persisted skip is
  // left untouched, so the automatic policy is unchanged.
  if (!force && skipped !== null && normalizeVersion(skipped) === latest) {
    setStatus({ kind: 'skipped', current, latest });
    return;
  }
  setStatus({ kind: 'available', current, latest });
}

/**
 * Start the one check this session runs.
 *
 * Additive and idempotent: a second call (a React StrictMode double-effect, a
 * second mount) does nothing. The work is fire-and-forget; the UI observes the
 * store. Every failure becomes a status, never an unhandled rejection.
 */
export function startUpdateCheck(deps: UpdateDeps = {}): void {
  if (started) return;
  started = true;
  policyStorage = deps.storage;
  generation += 1;
  const run = generation;
  void runUpdateCheck(deps, run, false);
}

/**
 * Run a check the user asked for explicitly (the About dialog's action).
 *
 * It bypasses both the session guard and the 24 h throttle, and it reports a
 * release even when the user previously skipped it - that is the point of
 * pressing the button. The persisted skip and the recorded timestamp keep their
 * automatic-policy meaning; a newer run supersedes any check still in flight.
 */
export function forceUpdateCheck(deps: UpdateDeps = {}): void {
  started = true;
  if (deps.storage !== undefined) policyStorage = deps.storage;
  generation += 1;
  const run = generation;
  void runUpdateCheck(deps, run, true);
}

/** Dismiss the notice for this session (`available -> idle`); nothing else changes. */
export function dismissUpdate(): void {
  if (status.kind !== 'available') return;
  setStatus({ kind: 'idle' });
}

/**
 * Skip the offered release (`available -> skipped`) and persist it.
 *
 * The next check compares against the same normalized version, so the skipped
 * release stays suppressed; a newer release still produces a notice.
 */
export function skipUpdateVersion(): void {
  if (status.kind !== 'available') return;
  markSkipped(status.latest, policyStorage);
  setStatus({ kind: 'skipped', current: status.current, latest: status.latest });
}

/**
 * Ask the host (or, on this carrier, the transport) to open the download page.
 *
 * The WebView never navigates itself: the native hosts own the browser/OS
 * hand-off, and the browser transport uses `window.open`. A refusal is returned
 * verbatim, never swallowed into a success.
 */
export async function openReleasePage(
  deps: Pick<UpdateDeps, 'invoke'> = {},
): Promise<{ ok: true } | { ok: false; reason: string }> {
  const invokeFn = deps.invoke ?? hostInvoke;
  try {
    await invokeFn('cad_open_release_page');
    return { ok: true };
  } catch (error) {
    return { ok: false, reason: errorMessage(error) };
  }
}

/**
 * Test seam: forget the session guard and the current status.
 *
 * Bumps `generation`, so a check still in flight from before the reset is stale
 * and cannot publish over the next one. Production code never calls this.
 */
export function resetUpdateStoreForTests(): void {
  started = false;
  generation += 1;
  policyStorage = undefined;
  status = { kind: 'idle' };
  emit();
}
