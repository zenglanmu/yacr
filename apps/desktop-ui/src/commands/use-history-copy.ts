// The copy affordance's state, shared by both history presentations.
//
// One transient status — "Copied" or "Copy failed" — and one timer, so the
// desktop panel and the mobile sheet read the same way and neither can claim a
// copy that did not happen. The write itself lives in `clipboard.ts`; this hook
// only turns its boolean answer into a localized status that clears itself.
//
// The failure is the point: `writeClipboardText` returns `false` when every
// clipboard path failed, and the caller renders `command.history.copy_failed`
// from the catalog (never a silent no-op — AGENTS rule 2).
import { useCallback, useEffect, useRef, useState } from 'react';
import { writeClipboardText } from '../clipboard.ts';

/** What the header's `role="status"` shows; `null` is the quiet state. */
export type HistoryCopyStatus = 'copied' | 'failed' | null;

/** How long the status stays after a copy (the same beat as the chrome's hints). */
export const HISTORY_COPY_STATUS_MS = 1500;

/** One presentation's copy state: read `status`, call `copy`. */
export interface HistoryCopyController {
  readonly status: HistoryCopyStatus;
  copy(text: string): void;
}

/**
 * A transient copy status with its own timer.
 *
 * A later copy replaces an earlier one (including a stale async answer: the
 * request counter makes only the newest write able to set the status), and the
 * timer is cleared on unmount so an unmounted pane cannot set state.
 */
export function useHistoryCopy(): HistoryCopyController {
  const [status, setStatus] = useState<HistoryCopyStatus>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const request = useRef(0);

  const clearTimer = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => clearTimer, [clearTimer]);

  const copy = useCallback((text: string) => {
    const id = request.current + 1;
    request.current = id;
    clearTimer();
    void writeClipboardText(text)
      .catch(() => false)
      .then(copied => {
        // A slower earlier write must not overwrite the newest answer.
        if (id !== request.current) return;
        setStatus(copied ? 'copied' : 'failed');
        timer.current = setTimeout(() => {
          timer.current = null;
          setStatus(null);
        }, HISTORY_COPY_STATUS_MS);
      });
  }, [clearTimer]);

  return { status, copy };
}
