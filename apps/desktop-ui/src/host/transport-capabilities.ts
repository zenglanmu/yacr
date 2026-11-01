// The one transport capability overlay for the page carrier.
//
// A wasm CAD session has no DOM and no filesystem, so it reports `canOpen: false`
// — that is the truth about the **session**. This carrier is what actually owns
// the browser file picker and the drawing bytes, and hands them to the session's
// own import (`open_bytes`, see `host/web-adapter.ts`). Lifting the flag in one
// place keeps the two status paths agreeing: the `cad_status` invoke answer and
// every pushed `status-changed` frame. Without that, a push frame (which comes
// straight from the session) would reset the lift on every frame and the chrome
// would refuse `OPEN` again.
//
// Only `canOpen` is lifted. `canNew`/`canSave`/`canPlot`/`canFit` stay exactly
// what the session reported: this transport cannot serve those, so it must not
// invent them. A native host reports its own real `canOpen` and never comes here.

import type { CadStatus } from '../cad/cad-status.ts';

/**
 * The session's status as the page transport advertises it.
 *
 * The single source of truth for the overlay: `host/web-adapter.ts` (the
 * `cad_status` answer) and `cad/cad-events.ts` (the wasm push transport's
 * initial frame, every live frame, and the `shell_status` recovery read) both
 * call it. Tiny and side-effect free — it never mutates `status`; an already
 * lifted status is returned as-is.
 */
export function withTransportCapabilities(status: CadStatus): CadStatus {
  return status.canOpen ? status : { ...status, canOpen: true };
}
