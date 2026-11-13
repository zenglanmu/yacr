// WebView defaults the desktop shell must keep away from the CAD session.
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
//   surface at, fighting the "drawing area must match the DOM rect" guard;
// - the WebKit DOM pinch gestures (`gesturestart`/`gesturechange`/`gestureend`).
//
// Scope of these guards: they are a **DOM** safety net, not the Linux pinch fix.
// A trackpad pinch reaches this DOM as `Ctrl`+wheel on Chromium/WebView2 and as a
// WebKit gesture on Safari/WKWebView, so both are cancelled here; WebKitGTK
// instead handles a trackpad pinch in the **UI process** (`GtkGestureZoom` in
// `WebKitWebViewBase`) where no DOM event exists and `preventDefault()` cannot
// reach it. That carrier is handled natively, once, by
// `apps/app-linux/src/gtk_bridge/pinch.rs`; the CSS `touch-action: none` on
// `.native-input` covers the touchscreen two-contact path.
//
// The command line keeps its native menu (copy/paste must keep working), so
// text-entry targets are exempt from the context-menu suppression. Everything
// else is blocked here, in one place, and a right-click inside the CAD partition
// is handed to the single shortcut-menu hook below — which builds the menu model
// the app renders as an in-DOM overlay.
import { buildContextMenu } from './commands/context-menu.ts';
import type { ContextMenuModel, ContextMenuSelection, ContextMenuState } from './commands/context-menu.ts';
import type { CadStatus } from './cad/cad-status.ts';
import { pointerPosition } from './cad/metrics.ts';
import type { LocalPoint } from './cad/metrics.ts';

const TEXT_ENTRY_TAGS: ReadonlySet<string> = new Set(['INPUT', 'TEXTAREA']);

/** The browser command a key/wheel event would trigger (WebKit actions). */
export type BrowserCommand = 'reload' | 'print' | 'zoom-in' | 'zoom-out' | 'zoom-reset';

/** The key-event fields the accelerator table reads. */
export interface BrowserKeyEvent {
  readonly key?: string;
  readonly code?: string;
  readonly ctrlKey?: boolean;
}

/** The detail the one CAD shortcut-menu hook receives. */
export interface CadContextMenuDetail {
  /** CSS pixels relative to the CAD partition rect. */
  readonly point?: LocalPoint;
  readonly selection?: ContextMenuSelection | null;
  readonly tool?: object | null;
  readonly lastCommand?: string | null;
  readonly canFit?: boolean;
  readonly hasDocument?: boolean;
}

export interface BrowserDefaultsOptions {
  readonly cadSelector?: string;
  readonly context?: () => Omit<CadContextMenuDetail, 'point'>;
  readonly onContextMenu?: (detail: CadContextMenuDetail) => void;
  readonly onBlocked?: (command: string) => void;
}

/** The class of the input-transparent native CAD partition. */
export const CAD_PARTITION_SELECTOR = '.native-input';

/**
 * Whether a context-menu target is a text-entry surface that must keep the
 * webview's native menu (copy/paste, spellcheck, IME candidates).
 *
 * `input`/`textarea` match by tag name; a `contenteditable` host (or any child
 * of one) matches through `isContentEditable`/`closest`.
 */
export function isTextEntryTarget(target: unknown): boolean {
  if (!target || typeof target !== 'object') return false;
  const candidate = target as {
    tagName?: unknown;
    isContentEditable?: unknown;
    closest?: unknown;
  };
  const tag = typeof candidate.tagName === 'string' ? candidate.tagName.toUpperCase() : '';
  if (TEXT_ENTRY_TAGS.has(tag)) return true;
  if (candidate.isContentEditable === true) return true;
  return typeof candidate.closest === 'function'
    && Boolean((candidate.closest as (selector: string) => unknown)('[contenteditable]'));
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
export function browserCommandForKey(event: BrowserKeyEvent = {}): BrowserCommand | null {
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
export function isBrowserZoomWheel(event: { readonly ctrlKey?: boolean } = {}): boolean {
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
export function cadContextMenuRequest(detail: CadContextMenuDetail = {}): ContextMenuModel {
  return buildContextMenu({
    selection: detail.selection ?? null,
    tool: detail.tool ?? null,
    lastCommand: detail.lastCommand ?? null,
    canFit: detail.canFit === true,
    hasDocument: detail.hasDocument === true,
  });
}

/** The live facts a CAD right-click turns into the overlay's state. */
export interface CadContextMenuFacts {
  /** The latest `cad_status`, or `null` before the first answer. */
  readonly status: CadStatus | null;
  /** The command line's own history (newest last); there is no second log. */
  readonly history: readonly string[];
  /** The CAD partition element, or `null` while it is not mounted. */
  readonly node: Element | null;
}

/**
 * The overlay state for one CAD right-click.
 *
 * This is the app's callback body, kept here so `App.tsx` stays inside the code
 * gate: the hook reports its point relative to the partition, and the overlay is
 * viewport-fixed, so the partition rect is what converts it. `repeat` reads the
 * command line's own newest history entry, and `canFit`/`hasDocument` are the
 * same host facts the command-line admission rule uses. A missing node keeps the
 * press at its own coordinates (the menu still opens) instead of dropping it.
 */
export function cadContextMenuState(
  detail: CadContextMenuDetail,
  { status, history, node }: CadContextMenuFacts,
): ContextMenuState {
  const bounds = node && typeof node.getBoundingClientRect === 'function'
    ? node.getBoundingClientRect()
    : null;
  const point = detail.point ?? { x: 0, y: 0 };
  return {
    point: {
      x: (bounds?.left ?? 0) + point.x,
      y: (bounds?.top ?? 0) + point.y,
    },
    rect: bounds
      ? { x: bounds.left, y: bounds.top, width: bounds.width, height: bounds.height }
      : null,
    menu: cadContextMenuRequest({
      ...detail,
      lastCommand: history.at(-1) ?? null,
      canFit: status?.canFit === true,
      hasDocument: (status?.documentLabel ?? null) !== null,
    }),
  };
}

/**
 * Install the webview-default guards and return a disposer.
 *
 * `context` supplies the selection/tool slices carried by the CAD menu hook; it
 * is read lazily so a status poll never re-installs the listeners. `onBlocked`
 * is an optional observation seam (tests); production passes none, because a
 * blocked browser command is silent.
 */
export function bindBrowserDefaults(env: EventTarget, {
  cadSelector = CAD_PARTITION_SELECTOR,
  context = () => ({}),
  onContextMenu = cadContextMenuRequest,
  onBlocked = () => {},
}: BrowserDefaultsOptions = {}): { dispose(): void } {
  const contextMenu = (event: Event): void => {
    const mouse = event as MouseEvent;
    // The native menu stays available in text fields (copy/paste in the
    // command line and the tool text step).
    if (isTextEntryTarget(mouse.target)) return;
    mouse.preventDefault();
    const target = mouse.target as Element | null;
    const cad = target && typeof target.closest === 'function'
      ? target.closest(cadSelector)
      : null;
    // The one call site for the CAD shortcut menu. The default hook builds the
    // model from this detail; the app replaces it with a callback that renders
    // the overlay. Either way the press keeps reaching the input lane untouched:
    // only the webview's own menu is suppressed above.
    if (cad) onContextMenu({ point: pointerPosition(cad, mouse), ...context() });
  };
  const keydown = (event: Event): void => {
    const command = browserCommandForKey(event as KeyboardEvent);
    if (!command) return;
    event.preventDefault();
    onBlocked(command);
  };
  const wheel = (event: Event): void => {
    if (!isBrowserZoomWheel(event as WheelEvent)) return;
    event.preventDefault();
    onBlocked('zoom');
  };
  // WebKit's DOM pinch route (Safari/WKWebView): the page must never scale, even
  // though the CAD zoom itself is the host's job. On WebKitGTK these events do
  // not exist at all — that carrier's pinch is taken over natively.
  const gesture = (event: Event): void => {
    event.preventDefault();
    onBlocked('zoom');
  };
  env.addEventListener('contextmenu', contextMenu);
  env.addEventListener('keydown', keydown);
  env.addEventListener('wheel', wheel, { passive: false });
  env.addEventListener('gesturestart', gesture, { passive: false });
  env.addEventListener('gesturechange', gesture, { passive: false });
  env.addEventListener('gestureend', gesture);
  return {
    dispose(): void {
      env.removeEventListener('contextmenu', contextMenu);
      env.removeEventListener('keydown', keydown);
      env.removeEventListener('wheel', wheel);
      env.removeEventListener('gesturestart', gesture);
      env.removeEventListener('gesturechange', gesture);
      env.removeEventListener('gestureend', gesture);
    },
  };
}
