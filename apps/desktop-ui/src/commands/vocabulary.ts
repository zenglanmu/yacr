// The host's command vocabulary is fetched once at mount, but on a `dom-canvas`
// carrier the host can only answer after the wasm session module exists, and a
// cold load can fail *transiently* (a bootstrap fetch or instantiation error on a
// cold edge, an aborted wasm download). A single failed attempt used to leave the
// catalog empty for the life of the page, so every command reported "not
// supported" and the chrome looked unwired until a manual reload.
//
// Retrying here is a transport retry, never a fabricated success: nothing is
// invented, the same request is repeated, and after the last attempt the failure
// is returned unchanged so the chrome keeps the catalog empty and surfaces it.

/** Attempts for one vocabulary fetch (the first plus five retries). */
export const VOCABULARY_ATTEMPTS = 6;

/** Delay before the first retry, doubled per attempt. */
export const VOCABULARY_FIRST_DELAY_MS = 250;

/** Upper bound of one retry delay, so a slow recovery cannot stall for long. */
export const VOCABULARY_MAX_DELAY_MS = 4000;

/**
 * The delay before retry `attempt` (0-based), capped at
 * [`VOCABULARY_MAX_DELAY_MS`]: 250, 500, 1000, 2000, 4000, …
 */
export function vocabularyDelayMs(attempt: number): number {
  const step = Math.max(0, Math.floor(Number.isFinite(attempt) ? attempt : 0));
  return Math.min(VOCABULARY_FIRST_DELAY_MS * 2 ** step, VOCABULARY_MAX_DELAY_MS);
}

/** Knobs the tests drive; production uses the defaults. */
export interface VocabularyRetryOptions {
  readonly attempts?: number;
  /** The wait itself, injectable so a test never sleeps. */
  readonly wait?: (ms: number) => Promise<void>;
  /** Called with the 1-based number of every failed attempt. */
  readonly onRetry?: (attempt: number, error: unknown) => void;
}

/**
 * Run `request` until it succeeds or the attempt budget is spent.
 *
 * The last error is rethrown unchanged (no wrapping, no default value), so the
 * caller's failure path keeps its exact diagnostic.
 */
export async function loadCommandVocabulary(
  request: () => Promise<unknown>,
  options: VocabularyRetryOptions = {},
): Promise<unknown> {
  const attempts = Math.max(1, Math.floor(options.attempts ?? VOCABULARY_ATTEMPTS));
  const wait = options.wait ?? ((ms: number) => new Promise<void>(resolve => { setTimeout(resolve, ms); }));
  let last: unknown = null;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await request();
    } catch (error) {
      last = error;
      options.onRetry?.(attempt, error);
      if (attempt < attempts) await wait(vocabularyDelayMs(attempt - 1));
    }
  }
  throw last;
}
