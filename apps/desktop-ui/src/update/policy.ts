// The persisted update policy: when the last successful check ran, and which
// version the user asked never to be told about again.
//
// Storage is treated as hostile. Private modes throw on the `localStorage`
// property itself, quota errors throw on `setItem`, and a corrupt value is just a
// string. Every access below is wrapped, so none of it can turn startup into a
// crash; a blocked store simply means "check again next launch, forget the skip".

import { CHECK_INTERVAL_MS } from './config.ts';

/** The `Storage` surface this module needs (injectable for tests). */
export interface UpdateStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** Timestamp (ms since epoch) of the last **successful** manifest read. */
export const LAST_CHECK_KEY = 'yacr.update.lastCheckAt';

/** The normalized release the user chose to skip. */
export const SKIPPED_VERSION_KEY = 'yacr.update.skippedVersion';

/**
 * The page's `localStorage`, or `null` when it is absent or blocked.
 *
 * The property itself may throw (some privacy modes), which is why the read is
 * inside the `try` rather than only the method calls.
 */
export function defaultUpdateStorage(): UpdateStorage | null {
  try {
    const storage = (globalThis as { readonly localStorage?: UpdateStorage }).localStorage;
    return storage ?? null;
  } catch {
    return null;
  }
}

function read(storage: UpdateStorage | null, key: string): string | null {
  try {
    return storage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function write(storage: UpdateStorage | null, key: string, value: string): void {
  try {
    storage?.setItem(key, value);
  } catch {
    // Blocked or full storage: the in-memory session behaviour still applies.
  }
}

function remove(storage: UpdateStorage | null, key: string): void {
  try {
    storage?.removeItem(key);
  } catch {
    // Same as `write`: a blocked store is not an error the user must see.
  }
}

/**
 * The persisted last-check timestamp, or `null` when there is none (or it is not
 * a usable non-negative integer). `now` is accepted so callers can use one
 * timestamp for the whole decision, and is otherwise unused.
 */
export function lastCheckAt(
  _now?: number,
  storage: UpdateStorage | null = defaultUpdateStorage(),
): number | null {
  const raw = read(storage, LAST_CHECK_KEY);
  if (raw === null || raw.trim() === '') return null;
  const value = Number(raw);
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

/**
 * Persist a completed **successful** read. A failed fetch must not call this:
 * recording a failure as "checked" would suppress the retry for a whole day.
 */
export function markChecked(now: number, storage: UpdateStorage | null = defaultUpdateStorage()): void {
  if (!Number.isSafeInteger(now) || now < 0) return;
  write(storage, LAST_CHECK_KEY, String(now));
}

/** The release the user skipped, or `null` (also for a corrupt/empty value). */
export function skippedVersion(storage: UpdateStorage | null = defaultUpdateStorage()): string | null {
  const raw = read(storage, SKIPPED_VERSION_KEY);
  return raw !== null && raw.length > 0 ? raw : null;
}

/** Persist one skipped release (already normalized by the caller). */
export function markSkipped(version: string, storage: UpdateStorage | null = defaultUpdateStorage()): void {
  if (typeof version !== 'string' || version.length === 0) return;
  write(storage, SKIPPED_VERSION_KEY, version);
}

/** Forget the skipped release, so a later check may offer it again. */
export function clearSkipped(storage: UpdateStorage | null = defaultUpdateStorage()): void {
  remove(storage, SKIPPED_VERSION_KEY);
}

/**
 * Whether `now` is due for a manifest read: no previous successful check, or the
 * interval has elapsed. A future timestamp (clock skew) counts as "not yet".
 */
export function shouldCheck(now: number, storage: UpdateStorage | null = defaultUpdateStorage()): boolean {
  const last = lastCheckAt(now, storage);
  return last === null || now - last >= CHECK_INTERVAL_MS;
}
