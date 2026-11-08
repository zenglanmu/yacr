// Reading-surface facts for the history copy chord.
//
// `Ctrl+C` has two owners in this app: AutoCAD's `COPYCLIP` (`accelerators.ts`,
// handled on `window`) and, while the user is reading the expanded command
// history, the browser-style "copy what I am looking at". The chord handler must
// only take the press when the history pane really is the surface the user is
// reading; otherwise `COPYCLIP` keeps its documented meaning.
//
// This module answers only the DOM questions that decision needs — is focus
// inside the pane, is there a selection anchored inside it, what text is
// selected — so the two presentations (`CommandLine.tsx` and
// `MobileCommandBar.tsx`) cannot disagree about what "reading the history" means.
// Hover is the components' own fact (a pointer ref), because only the element
// that owns the pane can track enter/leave cheaply.
//
// Everything is defensive: a host without `getSelection`, or a detached node,
// answers "not reading" instead of throwing in the keydown handler.
/** What one history pane currently offers as a reading surface. */
export interface HistorySurfaceReading {
  /** Keyboard focus sits inside the pane. */
  readonly focused: boolean;
  /** The document selection is anchored inside the pane. */
  readonly selectionInside: boolean;
  /** The selected text, or `''` when the selection is empty or elsewhere. */
  readonly selectedText: string;
}

const NOT_READING: HistorySurfaceReading = Object.freeze({
  focused: false,
  selectionInside: false,
  selectedText: '',
});

/** A text node's parent is the element `contains` can answer for. */
function elementOf(node: Node | null): Node | null {
  if (!node) return null;
  return node.nodeType === 3 ? node.parentNode : node;
}

/**
 * The reading facts of `root`, or "not reading" when the pane is absent.
 *
 * `doc` is injectable for symmetry with `clipboard.ts`; the callers use the real
 * document, and no test mounts a DOM.
 */
export function historySurfaceReading(
  root: HTMLElement | null,
  doc: Document | null = typeof document === 'undefined' ? null : document,
): HistorySurfaceReading {
  if (!root || !doc) return NOT_READING;
  let focused = false;
  try {
    const active = doc.activeElement;
    focused = active !== null && root.contains(active);
  } catch {
    focused = false;
  }
  let selectionInside = false;
  let selectedText = '';
  try {
    const selection = typeof doc.getSelection === 'function' ? doc.getSelection() : null;
    if (selection && selection.rangeCount > 0) {
      const anchor = elementOf(selection.anchorNode);
      selectionInside = anchor !== null && root.contains(anchor);
      if (selectionInside && selection.isCollapsed !== true) selectedText = selection.toString();
    }
  } catch {
    selectionInside = false;
    selectedText = '';
  }
  return { focused, selectionInside, selectedText };
}
