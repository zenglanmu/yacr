// Quick View's pure projection: the open drawing(s) and the layout cards.
//
// The build is single-document (there is exactly one drawing per session), so
// Quick View Drawings has either one card or an honest empty state. The card
// never invents a thumbnail: `QuickViewPanel` paints a small vector diagram, and
// this module only names the drawing and says whether it is the current one.
//
// Quick View Layouts is the **same** `layoutTabsView` projection the strip under
// the drawing renders (`layout/layout-tabs.ts`): the cards name every real layout
// and dispatch the same `SPACE #<id>` / `MODEL` tokens, never a second parser.
import { layoutTabsView } from '../layout/layout-tabs.ts';
import type { CadStatus } from '../cad/cad-status.ts';

/** One drawing card. There is at most one in this build. */
export interface QuickViewDrawingCard {
  /** Stable key; the single document is always `current`. */
  readonly id: 'current';
  /** The host-reported label, or `null` when the open drawing is unnamed. */
  readonly label: string | null;
  /** Whether the label is absent (the panel names it `doctabs.untitled`). */
  readonly unnamed: boolean;
  readonly active: boolean;
}

/**
 * The drawing cards for the current session.
 *
 * `documentVisible` is the layout fact and `documentLabel` the document fact;
 * either one means the card exists (a `dom-canvas` session reports a label but
 * no host document). Neither means the panel shows its empty state instead of a
 * fabricated row.
 */
export function quickViewDrawings(
  documentLabel: string | null | undefined,
  documentVisible: boolean,
): readonly QuickViewDrawingCard[] {
  const label = typeof documentLabel === 'string' && documentLabel.trim().length > 0
    ? documentLabel
    : null;
  if (!documentVisible && label === null) return [];
  return Object.freeze([{ id: 'current', label, unnamed: label === null, active: true }]);
}

/** One layout card. Model space is a card too. */
export interface QuickViewLayoutCard {
  /** Stable key (`model` or `layout-<id>`). */
  readonly key: string;
  /** The paper layout's wire id, `null` for model space. */
  readonly id: number | null;
  /** The catalog key for the model card; `null` for a real layout. */
  readonly nameKey: string | null;
  /** The host's layout name, `null` for model space / an unnamed layout. */
  readonly name: string | null;
  readonly active: boolean;
  readonly supported: boolean;
  readonly viewportCount: number | null;
  /** The exact switch line, or `null` while this card is already active. */
  readonly command: string | null;
}

/**
 * Every layout card, in drawing order (model first).
 *
 * The cards carry the strip's own token: `MODEL` for model space and
 * `SPACE #<id>` for a paper layout. An unsupported layout stays visible but
 * disabled (`supported: false`), with no command to dispatch.
 */
export function quickViewLayouts(
  status: Partial<CadStatus> | null | undefined,
): readonly QuickViewLayoutCard[] {
  return layoutTabsView(status).map((tab): QuickViewLayoutCard => ({
    key: tab.id === null ? 'model' : `layout-${tab.id}`,
    id: tab.id,
    nameKey: tab.id === null ? 'layout.tab.model' : null,
    name: tab.name,
    active: tab.active,
    supported: tab.supported,
    viewportCount: tab.viewportCount,
    command: tab.command,
  }));
}
