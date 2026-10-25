// WebKitGTK defaults the desktop shell must keep away from the CAD session.
//
// The native CAD surface is deliberately input-transparent (its input region is
// empty, see `apps/app-linux/src/gtk_bridge/native.rs`), so *every* pointer and
// wheel event lands in this DOM. That also exposes the webview's own defaults:
//
// - the native context menu, whose `Reload` remounts the WebView while the Rust
//   session and the native CAD surface keep running (a state divergence);
// - reload (`Ctrl+R`, `Ctrl+Shift+R`, `F5`), print (`Ctrl+P`) and page zoom
//   (`Ctrl+=`/`Ctrl++`/`Ctrl+-`/`Ctrl+0`, `Ctrl`+wheel) accelerators. Page zoom
//   in particular changes the DOM rect/scale the host composites the CAD
//   surface at, fighting the "drawing area must match the DOM rect" guard.
//
// The command line keeps its native menu (copy/paste must keep working), so
// text-entry targets are exempt from the context-menu suppression. Everything
// else is blocked here, in one place, and a right-click inside the CAD partition
// is handed to the single shortcut-menu hook below — which builds the menu model
// the app renders as an in-DOM overlay.
import { buildContextMenu } from './context-menu.mjs';
import { pointerPosition } from './metrics.mjs';

const TEXT_ENTRY_TAGS = new Set(['INPUT', 'TEXTAREA']);

/** The class of the input-transparent native CAD partition. */
export const CAD_PARTITION_SELECTOR = '.native-input';

/**
 * Whether a context-menu target is a text-entry surface that must keep the
 * webview's native menu (copy/paste, spellcheck, IME candidates).
 *
 * `input`/`textarea` match by tag name; a `contenteditable` host (or any child
 * of one) matches through `isContentEditable`/`closest`.
 */
export function isTextEntryTarget(target) {
  if (!target || typeof target !== 'object') return false;
  const tag = typeof target.tagName === 'string' ? target.tagName.toUpperCase() : '';
  if (TEXT_ENTRY_TAGS.has(tag)) return true;
  if (target.isContentEditable === true) return true;
  return typeof target.closest === 'function' && Boolean(target.closest('[contenteditable]'));
}

/**
 * The browser command a key event would trigger, or `null` for every key the
 * app (or ordinary text editing) owns.
 *
 * Matching is on the *browser command*, never on focus: these are WebKit
 * actions, not text editing, so a focused text field does not exempt them.
 * Ordinary typing has no `ctrlKey`, and Enter/Escape/IME composition are not in
 * the table at all, so they are never swallowed here.
 */
export function browserCommandForKey(event = {}) {
  const key = typeof event.key === 'string' ? event.key : '';
  const code = typeof event.code === 'string' ? event.code : '';
  if (key === 'F5') return 'reload';
  if (!event.ctrlKey) return null;
  const lowered = key.toLowerCase();
  if (lowered === 'r') return 'reload';
  if (lowered === 'p') return 'print';
  if (key === '=' || key === '+' || code === 'Equal' || code === 'NumpadAdd') return 'zoom-in';
  if (key === '-' || key === '_' || code === 'Minus' || code === 'NumpadSubtract') return 'zoom-out';
  if (key === '0' || key === ')' || code === 'Digit0' || code === 'Numpad0') return 'zoom-reset';
  return null;
}

/** Whether a wheel event is WebKit's `Ctrl`+wheel page zoom. */
export function isBrowserZoomWheel(event = {}) {
  return event.ctrlKey === true;
}

/**
 * The single CAD shortcut-menu hook (right-click inside the CAD partition).
 *
 * It is the **pure adapter** between the DOM event and the menu model: it maps
 * the detail into [`buildContextMenu`] and returns the model, performing no DOM,
 * IPC or session work of its own. A caller that installs no `onContextMenu`
 * therefore gets the honest menu structure and nothing else.
 *
 * The app installs its own callback instead: same detail, same model, plus the
 * two things only the app knows — the partition rect (to convert `point` into
 * viewport CSS px for the fixed overlay) and the overlay's open/close state. The
 * press itself is never prevented from reaching the native input lane here.
 *
 * @param {{point: {x: number, y: number}, selection: object|null, tool: object|null,
 *          lastCommand?: string|null, canFit?: boolean, hasDocument?: boolean}} detail
 *   `point` is CSS pixels relative to the CAD partition rect; `selection`, `tool`,
 *   `lastCommand`, `canFit` and `hasDocument` are the state slices the UI already
 *   has (absent slices mean "not available", never "enabled by default").
 */
export function cadContextMenuRequest(detail = {}) {
  return buildContextMenu({
    selection: detail.selection ?? null,
    tool: detail.tool ?? null,
    lastCommand: detail.lastCommand ?? null,
    canFit: detail.canFit === true,
    hasDocument: detail.hasDocument === true,
  });
}

/**
 * Install the webview-default guards and return a disposer.
 *
 * `context` supplies the selection/tool slices carried by the CAD menu hook; it
 * is read lazily so a status poll never re-installs the listeners. `onBlocked`
 * is an optional observation seam (tests); production passes none, because a
 * blocked browser command is silent.
 */
export function bindBrowserDefaults(env, {
  cadSelector = CAD_PARTITION_SELECTOR,
  context = () => ({}),
  onContextMenu = cadContextMenuRequest,
  onBlocked = () => {},
} = {}) {
  const contextMenu = event => {
    // The native menu stays available in text fields (copy/paste in the
    // command line and the tool text step).
    if (isTextEntryTarget(event.target)) return;
    event.preventDefault();
    const cad = typeof event.target?.closest === 'function'
      ? event.target.closest(cadSelector)
      : null;
    // The one call site for the CAD shortcut menu. The default hook builds the
    // model from this detail; the app replaces it with a callback that renders
    // the overlay. Either way the press keeps reaching the input lane untouched:
    // only the webview's own menu is suppressed above.
    if (cad) onContextMenu({ point: pointerPosition(cad, event), ...context() });
  };
  const keydown = event => {
    const command = browserCommandForKey(event);
    if (!command) return;
    event.preventDefault();
    onBlocked(command);
  };
  const wheel = event => {
    if (!isBrowserZoomWheel(event)) return;
    event.preventDefault();
    onBlocked('zoom');
  };
  env.addEventListener('contextmenu', contextMenu);
  env.addEventListener('keydown', keydown);
  env.addEventListener('wheel', wheel, { passive: false });
  return {
    dispose() {
      env.removeEventListener('contextmenu', contextMenu);
      env.removeEventListener('keydown', keydown);
      env.removeEventListener('wheel', wheel);
    },
  };
}
