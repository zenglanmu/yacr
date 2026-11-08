// Writing plain text to the **system clipboard** (the command history's copy
// affordance).
//
// Two carriers, one call: the desktop shells are WebKitGTK/WKWebView/WebView2
// windows and the web build is a browser tab, so this uses the platform's own
// clipboard and nothing else. No Tauri clipboard plugin is added — the capability
// file stays `core`/`event` only, and a plugin would be a second permission
// surface for one button.
//
// Two strategies, in this order:
//
//   1. `navigator.clipboard.writeText` — the async Clipboard API. It needs a
//      secure context and a user gesture; when the context is not secure
//      (`env.isSecureContext === false`) it is skipped rather than called and
//      rejected.
//   2. A temporary off-screen `<textarea>` + `document.execCommand('copy')` —
//      the pre-Clipboard-API path, still the only one available in some WebKit
//      builds. The textarea is removed and the previous focus/selection put back,
//      so the history panel does not lose the reader's place.
//
// The answer is **honest**: `true` only when one path reported success; `false`
// when both were unavailable or failed. The caller shows a localized failure
// status — this module never pretends a copy happened and never swallows the
// difference (AGENTS rule 2).
//
// `env` exists so the fallback logic is unit-testable under plain Node: the
// suites inject a fake navigator/document and never touch a real clipboard.

/** The one method this module needs from a clipboard object. */
export interface ClipboardWriter {
  writeText(text: string): Promise<unknown> | unknown;
}

/** The navigator surface the copy path reads (structural: a real `Navigator` fits). */
export interface ClipboardNavigatorLike {
  readonly clipboard?: ClipboardWriter | null;
}

/**
 * Injectable environment. Every field is optional: the defaults are the real
 * browser globals when they exist.
 */
export interface ClipboardEnvironment {
  /** Defaults to the global `navigator`. */
  readonly navigator?: ClipboardNavigatorLike | null;
  /** Defaults to the global `document`. */
  readonly document?: Document | null;
  /** Overrides the global `window.isSecureContext` (tests, and hosts that lie). */
  readonly isSecureContext?: boolean;
}

/** What the fallback restored around the temporary textarea. */
interface SavedSelection {
  readonly selection: Selection | null;
  readonly ranges: readonly Range[];
}

function readNavigator(env: ClipboardEnvironment): ClipboardNavigatorLike | null {
  if (env.navigator !== undefined) return env.navigator;
  return typeof navigator === 'undefined' ? null : navigator;
}

function readDocument(env: ClipboardEnvironment): Document | null {
  if (env.document !== undefined) return env.document;
  return typeof document === 'undefined' ? null : document;
}

function readSecureContext(env: ClipboardEnvironment): boolean | undefined {
  if (env.isSecureContext !== undefined) return env.isSecureContext;
  return typeof window === 'undefined' ? undefined : window.isSecureContext;
}

/** The current selection, so the fallback can put it back after the copy. */
function saveSelection(doc: Document): SavedSelection {
  try {
    const selection = typeof doc.getSelection === 'function' ? doc.getSelection() : null;
    if (!selection) return { selection: null, ranges: [] };
    const ranges: Range[] = [];
    for (let index = 0; index < selection.rangeCount; index += 1) ranges.push(selection.getRangeAt(index));
    return { selection, ranges };
  } catch {
    // A host that refuses selection access is not a reason to skip the copy.
    return { selection: null, ranges: [] };
  }
}

function restoreSelection(saved: SavedSelection): void {
  const { selection, ranges } = saved;
  if (!selection) return;
  try {
    selection.removeAllRanges();
    for (const range of ranges) selection.addRange(range);
  } catch {
    // Best effort: the text is already on the clipboard.
  }
}

/**
 * The `<textarea>` + `execCommand('copy')` path.
 *
 * `false` covers every reason the path could not complete — no body, no
 * `execCommand`, a throwing host, or `execCommand` answering `false`. The
 * temporary node never survives the call, and focus/selection are restored even
 * on failure.
 */
function copyWithTextarea(text: string, doc: Document | null): boolean {
  if (!doc || !doc.body || typeof doc.execCommand !== 'function') return false;
  const previousFocus = doc.activeElement;
  const previousSelection = saveSelection(doc);
  let area: HTMLTextAreaElement | null = null;
  try {
    area = doc.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.setAttribute('aria-hidden', 'true');
    area.style.position = 'fixed';
    area.style.top = '-1000px';
    area.style.left = '-1000px';
    area.style.opacity = '0';
    doc.body.appendChild(area);
    // Focus first: WebKit only copies the *focused* control's selection on some
    // builds. Off-screen placement keeps this from scrolling the panel.
    if (typeof area.focus === 'function') area.focus({ preventScroll: true });
    if (typeof area.select === 'function') area.select();
    if (typeof area.setSelectionRange === 'function') area.setSelectionRange(0, text.length);
    return doc.execCommand('copy') === true;
  } catch {
    return false;
  } finally {
    const parent = area?.parentNode;
    if (area && parent) {
      try {
        parent.removeChild(area);
      } catch {
        // Already gone; nothing left to clean up.
      }
    }
    if (previousFocus && typeof (previousFocus as HTMLElement).focus === 'function') {
      try {
        (previousFocus as HTMLElement).focus({ preventScroll: true });
      } catch {
        // The node may have unmounted while the copy ran.
      }
    }
    restoreSelection(previousSelection);
  }
}

/**
 * Copy `text` to the system clipboard. `true` only on a reported success.
 *
 * An empty string is `false`: there is nothing to copy, and "Copied" over an
 * empty clipboard would be a claim nobody can check.
 */
export async function writeClipboardText(text: string, env: ClipboardEnvironment = {}): Promise<boolean> {
  if (typeof text !== 'string' || text === '') return false;
  const clipboard = readNavigator(env)?.clipboard;
  if (clipboard && typeof clipboard.writeText === 'function' && readSecureContext(env) !== false) {
    try {
      await clipboard.writeText(text);
      return true;
    } catch {
      // Denied, insecure context, or no gesture: fall through to the legacy path
      // instead of reporting a failure the user cannot act on.
    }
  }
  return copyWithTextarea(text, readDocument(env));
}
