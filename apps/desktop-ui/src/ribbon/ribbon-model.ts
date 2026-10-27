// Declarative ribbon definition. Pure data (no React), so both the renderer and
// the tests can walk it. An item carries exactly one dispatch token:
//
//   * `command` — a payload-free name admitted through `cad_command`;
//   * `standardView` — one of the seven host view keys, which carries a payload
//     and therefore travels on the dedicated typed IPC `cad_standard_view`
//     (never on `cad_command`);
//   * `action` — a DOM-only shell action that never touches the CAD protocol.
//
// An item with none of the three is labelled but disabled, which is the honest
// state for a command the host does not wire yet.
import { POPUP_KEYS } from '../popup/popup.ts';
import { OPTIONS_KEYS } from '../options/options.ts';
import { STANDARD_VIEWS } from '../cad/view3d.ts';

/** Shell actions that never touch the CAD protocol. */
export const SHELL_ACTIONS = Object.freeze({
  diagnostics: 'diagnostics',
  cleanScreen: 'cleanScreen',
  commandLine: 'commandLine',
  language: 'language',
  about: 'about',
  // Opens the Options dialog: a host-owned window like About, so it is an
  // `action` token and never a router command (`OPTIONS` has no Rust command;
  // the command line intercepts it locally).
  options: 'options',
  // Toggles the docked properties palette. Like OPTIONS it is chrome-local:
  // `PROPERTIES` has no Rust command, so the ribbon must not send `cad_command`.
  properties: 'properties',
});

/** The shell actions that toggle a docked palette. */
export const PALETTE_ACTIONS: readonly string[] = Object.freeze([
  SHELL_ACTIONS.diagnostics,
  SHELL_ACTIONS.properties,
]);

/**
 * Palette toggles that dispatch a **command name** instead of a shell action.
 *
 * `LAYER` opens the layers palette but rides the command funnel (it is a
 * chrome-local selector: the palette answers it, no Rust command exists), so its
 * dispatch token is `command` even though the affordance is a palette toggle. For
 * this carrier filter the two spellings mean the same thing.
 *
 * `LAYISO`/`LAYUNISO` are **router** commands, not palette toggles, and they are
 * listed here only for the carrier rule: layer isolation is a state the layers
 * palette is the only surface for, so on a carrier that never renders the palette
 * (`dom-canvas`) the isolate pair would hide geometry with nothing on screen to
 * explain or undo it — a dead affordance, exactly like the toggle itself. They
 * are therefore dropped with the palette rather than left behind as live buttons.
 *
 * The rule this list encodes is therefore not "is it a palette toggle" but
 * **"is its effect only observable in the layers palette"**. `LAYOFF`/`LAYON`
 * (2026-10-26) pass it for the same reason: turning the selection's layers off
 * hides geometry, and turning every layer back on restores it, and the palette is
 * the only place either state is named or reversed. Left behind on a palette-less
 * carrier they would hide geometry with no surface to explain it, so they are
 * dropped with the palette too.
 */
export const PALETTE_COMMANDS: readonly string[] = Object.freeze([
  'LAYER',
  'LAYISO',
  'LAYUNISO',
  // `LAYERP` (2026-10-27) restores the previous layer-override state: the result is
  // again a layer state that only the palette names or reverses, so it is dropped
  // with the palette for the same reason as the rest of the layer run.
  'LAYERP',
  'LAYOFF',
  'LAYON',
]);

/** The facts every ribbon item carries. */
interface RibbonItemBase {
  readonly id: string;
  readonly labelKey: string;
  readonly icon: string;
  readonly large?: boolean;
  /** The item's own popover (a small caret); its entries are items too. */
  readonly variants?: readonly RibbonItem[];
}

/** One item's dispatch token: exactly one of the three families. */
export type RibbonItemKind = 'command' | 'standardView' | 'action';

/** A payload-free router command, admitted through `cad_command`. */
export interface RibbonCommandItem extends RibbonItemBase {
  readonly command: string;
  readonly standardView?: never;
  readonly action?: never;
}

/** One of the seven standard views, sent on `cad_standard_view { view }`. */
export interface RibbonStandardViewItem extends RibbonItemBase {
  readonly standardView: string;
  readonly command?: never;
  readonly action?: never;
}

/** A DOM-only shell action that never touches the CAD protocol. */
export interface RibbonActionItem extends RibbonItemBase {
  readonly action: string;
  readonly command?: never;
  readonly standardView?: never;
}

/**
 * One ribbon button.
 *
 * The three dispatch tokens are a union rather than three optional fields: an
 * item that carried two of them would have no defined order, and `Ribbon.jsx`'s
 * dispatch checks one family at a time.
 */
export type RibbonItem = RibbonCommandItem | RibbonStandardViewItem | RibbonActionItem;

/** One panel of a tab. */
export interface RibbonPanel {
  readonly id: string;
  readonly titleKey: string;
  readonly items: readonly RibbonItem[];
  /** The panel's dialog-launcher arrow exists (disabled when it has no target). */
  readonly launcher?: boolean;
  /** The chrome-local selector the launcher arrow runs (see [`panelLauncherCommand`]). */
  readonly launcherCommand?: string;
  /** Small-item grid rows (column-major); the layout contract is data. */
  readonly rows?: number;
}

/** One tab of the ribbon. */
export interface RibbonTab {
  readonly id: string;
  readonly labelKey: string;
  readonly accent?: boolean;
  readonly panels: readonly RibbonPanel[];
}

/** One cell of a column-major panel grid. */
export interface RibbonCell<T> {
  readonly item: T;
  readonly column: number;
  readonly row: number;
}

/**
 * The items a panel actually offers on the reported carrier.
 *
 * A docked palette the carrier does not have must not be offered anywhere: the
 * status-bar chips are hidden, and so are these ribbon entries, because a button
 * that toggles a panel which is never rendered is a dead affordance. Only the
 * palette toggles are filtered; every other item is returned as the very same
 * array, so a carrier with the palettes (`native-below`, and any chrome state
 * without the flag) renders exactly the ribbon it always did.
 */
export function offeredItems(
  items: readonly RibbonItem[],
  palettesAvailable: boolean | undefined,
): readonly RibbonItem[] {
  if (palettesAvailable !== false) return items;
  return items.filter(item => !(typeof item.action === 'string' && PALETTE_ACTIONS.includes(item.action))
    && !(typeof item.command === 'string' && PALETTE_COMMANDS.includes(item.command)));
}

/**
 * The panels a tab actually offers on the reported carrier.
 *
 * Filtering the items is not enough: the Home tab's "Layers" and "Properties"
 * panels hold nothing but palette toggles, so on a carrier without the palettes
 * they would render as empty titled boxes. A panel that has nothing left is
 * dropped whole, together with its title and launcher.
 */
export function offeredPanels(
  panels: readonly RibbonPanel[],
  palettesAvailable: boolean | undefined,
): readonly RibbonPanel[] {
  if (palettesAvailable !== false) return panels;
  return panels.filter(panel => offeredItems(panel.items, palettesAvailable).length > 0);
}

/**
 * The command a panel's dialog-launcher arrow runs, or `null`.
 *
 * The arrow in a panel's title row is the convention for "deeper settings of
 * this panel". A panel declares its target as data (`launcherCommand`), so the
 * renderer never has to know panel ids:
 *
 *   * `'LAYER'` / `'PROPERTIES'` — the panel's own palette, opened (and, for
 *     layers, focused) by the chrome-local selector of that name. Neither is a
 *     Rust command, so neither needs a document or a router entry;
 *   * no token (or `null`) — no dialog exists for that panel yet. The arrow stays
 *     visible but disabled with the catalog key `limits.ts` holds
 *     (`ribbon.panel_dialog_missing`), which says exactly that instead of
 *     claiming the command is unsupported.
 *
 * Anything that is not a non-empty string is "no target": an empty token would
 * otherwise resolve to a command name of `''` and quietly render a live arrow.
 */
export function panelLauncherCommand(panel: RibbonPanel | null | undefined): string | null {
  const command = panel?.launcherCommand ?? null;
  return typeof command === 'string' && command.length > 0 ? command : null;
}

export const RIBBON_TABS: readonly RibbonTab[] = Object.freeze<RibbonTab[]>([
  {
    id: 'file',
    labelKey: 'ribbon.file',
    accent: true,
    panels: [
      {
        id: 'file',
        titleKey: 'ribbon.file',
        items: [
          { id: 'OPEN', labelKey: 'file.open', icon: 'open', command: 'OPEN', large: true },
          { id: 'NEW', labelKey: 'qat.new', icon: 'new', command: 'NEW', large: true },
          { id: 'SAVE', labelKey: 'qat.save', icon: 'save', command: 'SAVE', large: true },
          // Plot is a file-level output action like Save: it is the same router
          // selector as the `EXPORT`/`PRINT` aliases, gated by `canPlot`. The
          // entry reuses the `save` glyph (there is no dedicated plot icon).
          { id: 'PLOT', labelKey: 'file.plot', icon: 'save', command: 'PLOT', large: true },
        ],
      },
      {
        id: 'utilities',
        titleKey: 'ribbon.panel.utilities',
        items: [
          { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics' },
          { id: 'clean', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: 'cleanScreen' },
          { id: 'cmdline', labelKey: 'status.command_line', icon: 'terminal', action: 'commandLine' },
          // About opens a host-owned dialog window (ADR 0021 D1), so it is an
          // `action` token, not a router command: nothing about it goes through
          // `cad_command`. It closes the File tab's utility cluster, where the
          // other chrome-level affordances live.
          { id: 'about', labelKey: POPUP_KEYS.ribbonEntry, icon: 'app', action: 'about' },
          // Options sits beside About: the other host-owned settings window.
          // Its label is the dialog's own title key, so the ribbon entry and the
          // window title can never drift apart.
          { id: 'options', labelKey: OPTIONS_KEYS.ribbonEntry, icon: 'settings', action: 'options' },
        ],
      },
    ],
  },
  {
    id: 'home',
    labelKey: 'ribbon.home',
    panels: [
      {
        id: 'clipboard',
        titleKey: 'ribbon.panel.clipboard',
        items: [
          { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL', large: true },
          { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
          { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
          { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
        ],
      },
      {
        id: 'draw',
        titleKey: 'ribbon.panel.draw',
        items: [
          { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
          { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE', large: true },
          { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC' },
          { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
          { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
          { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
          { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
          { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', command: 'POINT' },
          { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
          { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
          // Single-line TEXT (ADR 0019 decision A). It is a small button next to
          // OFFSET, before HATCH: like HATCH it is not a primary creation tool,
          // and it has no variant popover — the command itself opens the
          // command-line text step (there is no in-place canvas editor here).
          { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
          // MTEXT (2026-10-26) is the multiline counterpart and sits directly
          // beside TEXT: two opposite corners (first = insertion point, opposite =
          // wrap width) and then the same terminal text step. Like TEXT it is a
          // small button with **no variant popover** — rotation/attachment/line
          // spacing/columns/background are NOT implemented, so a caret here would
          // imply choices the command does not have.
          { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT' },
          // MLEADER (ADR 0019 decision F.2) is the other text-carrying
          // annotation tool, so it sits directly beside TEXT and before HATCH. It
          // captures exactly two points — the arrowhead position, then the
          // landing point — and then the same terminal text step TEXT uses. It is
          // a small button with **no variant popover on purpose**: no `Options`
          // sub-prompt exists (leader type / content type / max points / angular
          // constraint), and a caret here would imply choices the command does
          // not have.
          { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
          // HATCH closes the draw group, the way AutoCAD's Draw panel puts Hatch
          // at its end. It is a small button: unlike LINE/CIRCLE it is not a
          // primary creation tool, and it has no variant popover (the unimplemented
          // pattern-name/gradient forms must not appear as menu entries).
          { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
        ],
      },
      {
        id: 'modify',
        titleKey: 'ribbon.panel.modify',
        items: [
          { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM', large: true },
          { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
          { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
          // MATCHPROP (MA) is the selection-based property painter, so it sits
          // beside ERASE/EXPLODE — the panel's other commands that act on the
          // **current** selection. It is a small button with no variant popover:
          // there is no Settings sub-prompt (which properties to copy), the
          // command always transfers all five display properties, and a caret
          // here would imply choices the command does not have. Its enabled state
          // is the shared admission rule plus its own selection requirement
          // (`SELECTION_REQUIREMENTS`), so with fewer than two selected objects it
          // reads disabled with the sentence that names the rule.
          { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
          { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
          { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
          { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
          // The Polyline modes sit directly after the two-pick pair they belong
          // to: the same corner treatment, one preselected open polyline instead
          // of two picks. Small buttons with no variant popover (there is no
          // options sub-prompt and no TRIMMODE switch), enabled by the shared
          // admission plus their own one-object selection requirement.
          { id: 'CHAMFER POLYLINE', labelKey: 'draw.kind.chamfer_polyline', icon: 'chamfer', command: 'CHAMFER POLYLINE' },
          { id: 'FILLET POLYLINE', labelKey: 'draw.kind.fillet_polyline', icon: 'fillet', command: 'FILLET POLYLINE' },
          { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
          { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
          { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
          { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
          { id: 'MIRROR', labelKey: 'draw.kind.mirror', command: 'MIRROR', icon: 'mirror' },
          { id: 'SCALE', labelKey: 'draw.kind.scale', command: 'SCALE', icon: 'scale' },
          // JOIN (2026-10-26) closes the Modify panel the way AutoCAD's does. It
          // is a small button (it is not a primary edit) with no variant popover —
          // the command has no options sub-prompt — and its enablement is the
          // shared admission **plus** its own two-object selection requirement, so
          // with fewer objects it reads disabled with the rule's sentence.
          { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', command: 'JOIN' },
        ],
      },
      {
        id: 'layers',
        titleKey: 'layers.panel',
        // The arrow in this panel's title row opens the same surface the LAYER
        // button does: the docked layers palette (`LAYER` is chrome-local, so a
        // repeat press focuses and re-reads it rather than opening a second one).
        launcher: true,
        launcherCommand: 'LAYER',
        items: [
          { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER', large: true },
          // Layer isolate / unisolate (2026-10-26) sit beside the palette opener
          // they operate on: both are payload-free router commands acting on the
          // **current selection** (LAYISO needs at least one object, LAYUNISO
          // none), so they are small buttons with the shared admission and no
          // popover. They are dropped with the palette on a carrier that never
          // renders it (`PALETTE_COMMANDS`), because the isolation state is only
          // visible there.
          { id: 'LAYISO', labelKey: 'layers.isolate', icon: 'eyeOff', command: 'LAYISO' },
          { id: 'LAYUNISO', labelKey: 'layers.unisolate', icon: 'eye', command: 'LAYUNISO' },
          // LAYERP (2026-10-27) restores the previous layer-override state, so it
          // sits directly beside LAYUNISO — the two are the layer run's two
          // "bring back what was there" commands, and the pair reads in the order
          // the reader reaches for them (unisolate everything, or go back one
          // step). It is a payload-free router command with no popover, and the
          // carrier rule is the panel's: the state it restores is only visible in
          // the palette, so it is dropped with it (`PALETTE_COMMANDS`).
          { id: 'LAYERP', labelKey: 'layers.previous', icon: 'eye', command: 'LAYERP' },
          // Layer off / on (2026-10-26) close the panel: LAYOFF turns off every
          // layer the selection sits on (needs one object), LAYON turns every
          // layer back on (needs none). Same conventions as the isolate pair —
          // small, payload-free router commands, no popover — and the same
          // carrier rule: the resulting layer state is only visible in the
          // palette, so they are dropped with it (`PALETTE_COMMANDS`).
          { id: 'LAYOFF', labelKey: 'draw.kind.layoff', icon: 'eyeOff', command: 'LAYOFF' },
          { id: 'LAYON', labelKey: 'draw.kind.layon', icon: 'eye', command: 'LAYON' },
        ],
      },
      {
        // Properties sits in its own panel, the way AutoCAD's Home tab keeps
        // Layers and Properties apart: each palette then has one launcher that
        // opens exactly that palette. The panel is a palette toggle only, so a
        // carrier without the palettes drops it whole (`offeredPanels`).
        id: 'properties',
        titleKey: 'properties.panel',
        launcher: true,
        launcherCommand: 'PROPERTIES',
        items: [
          { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', action: 'properties', large: true },
        ],
      },
      {
        id: 'navigate',
        titleKey: 'ribbon.panel.navigate',
        items: [
          {
            id: 'FIT',
            labelKey: 'toolbar.fit',
            icon: 'fit',
            command: 'FIT',
            large: true,
            variants: [
              { id: 'FIT', labelKey: 'toolbar.zoom_extents', icon: 'fit', command: 'FIT' },
              { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
              { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
            ],
          },
          { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
          { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
        ],
      },
    ],
  },
  {
    id: 'draw',
    labelKey: 'ribbon.panel.draw',
    panels: [
      {
        id: 'draw',
        titleKey: 'ribbon.panel.draw',
        items: [
          { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', command: 'LINE', large: true },
          { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', command: 'CIRCLE', large: true },
          { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', command: 'ARC', large: true },
          { id: 'PLINE', labelKey: 'draw.kind.polyline', icon: 'polyline', command: 'PLINE' },
          { id: 'RECTANG', labelKey: 'draw.kind.rectang', icon: 'rectang', command: 'RECTANG' },
          { id: 'ELLIPSE', labelKey: 'draw.kind.ellipse', icon: 'ellipse', command: 'ELLIPSE' },
          { id: 'POLYGON', labelKey: 'draw.kind.polygon', icon: 'polygon', command: 'POLYGON' },
          { id: 'POINT', labelKey: 'draw.kind.point', icon: 'point', command: 'POINT' },
          { id: 'SPLINE', labelKey: 'draw.kind.spline', icon: 'spline', command: 'SPLINE' },
          { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', command: 'OFFSET' },
          // Same draw-group tail on the Draw tab: TEXT and MTEXT, then MLEADER
          // next to them, then Hatch last — the small-button decisions match the
          // Home panel.
          { id: 'TEXT', labelKey: 'draw.kind.text', icon: 'text', command: 'TEXT' },
          { id: 'MTEXT', labelKey: 'draw.kind.mtext', icon: 'mtext', command: 'MTEXT' },
          { id: 'MLEADER', labelKey: 'draw.kind.mleader', icon: 'mleader', command: 'MLEADER' },
          { id: 'HATCH', labelKey: 'draw.kind.hatch', icon: 'hatch', command: 'HATCH' },
        ],
      },
      {
        id: 'modify',
        titleKey: 'ribbon.panel.modify',
        launcher: true,
        items: [
          { id: 'ERASE', labelKey: 'draw.kind.erase', icon: 'erase', command: 'ERASE' },
          { id: 'EXPLODE', labelKey: 'draw.kind.explode', icon: 'explode', command: 'EXPLODE' },
          // Same slot as on the Home tab: directly after the other selection-acting
          // commands (see the Home/modify comment for why it is a small button with
          // no popover and why its enablement is selection-gated).
          { id: 'MATCHPROP', labelKey: 'draw.kind.matchprop', icon: 'matchprop', command: 'MATCHPROP' },
          { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', command: 'MOVE' },
          { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', command: 'COPY' },
          { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', command: 'TRIM' },
          { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', command: 'EXTEND' },
          { id: 'CHAMFER', labelKey: 'draw.kind.chamfer', icon: 'chamfer', command: 'CHAMFER' },
          { id: 'FILLET', labelKey: 'draw.kind.fillet', icon: 'fillet', command: 'FILLET' },
          // The Polyline modes sit directly after the two-pick pair they belong
          // to: the same corner treatment, one preselected open polyline instead
          // of two picks. Small buttons with no variant popover (there is no
          // options sub-prompt and no TRIMMODE switch), enabled by the shared
          // admission plus their own one-object selection requirement.
          { id: 'CHAMFER POLYLINE', labelKey: 'draw.kind.chamfer_polyline', icon: 'chamfer', command: 'CHAMFER POLYLINE' },
          { id: 'FILLET POLYLINE', labelKey: 'draw.kind.fillet_polyline', icon: 'fillet', command: 'FILLET POLYLINE' },
          { id: 'ARRAYRECT', labelKey: 'draw.kind.arrayrect', icon: 'arrayrect', command: 'ARRAYRECT' },
          { id: 'ARRAYPOLAR', labelKey: 'draw.kind.arraypolar', icon: 'arraypolar', command: 'ARRAYPOLAR' },
          { id: 'ALIGN', labelKey: 'draw.kind.align', icon: 'align', command: 'ALIGN' },
          { id: 'STRETCH', labelKey: 'draw.kind.stretch', icon: 'stretch', command: 'STRETCH' },
          { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', command: 'ROTATE' },
          { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', command: 'SCALE' },
          { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', command: 'MIRROR' },
          // Same slot as on the Home tab: JOIN closes the Modify panel (AutoCAD's
          // own order puts Join last), small, no popover, selection-gated.
          { id: 'JOIN', labelKey: 'draw.kind.join', icon: 'polyline', command: 'JOIN' },
        ],
      },
    ],
  },
  {
    id: 'measure',
    labelKey: 'ribbon.measure',
    panels: [
      {
        id: 'measure',
        titleKey: 'measure.panel',
        launcher: true,
        items: [
          { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', command: 'DIST', large: true },
          { id: 'PLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', command: 'MEASUREPOLYLINE', large: true },
          { id: 'ANGLE', labelKey: 'measure.kind.angle', icon: 'angle', command: 'MEASUREANGLE', large: true },
          {
            id: 'AREA',
            labelKey: 'measure.kind.area',
            icon: 'area',
            command: 'AREA',
            large: true,
            // `AREA OBJECT` (2026-10-27) is AutoCAD's Area → Object mode: the
            // current selection, one closed object, measured analytically. It is a
            // popover row rather than its own button because AutoCAD keeps it in
            // the same split button. A variant row is enabled by
            // `shell.canCommand('AREA OBJECT')`, which works now that admission
            // resolves the whole line before the base-plus-argument grammar — the
            // multi-word selector used to be refused locally, which is exactly why
            // this row could not exist before.
            variants: [
              { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', command: 'AREA' },
              {
                id: 'AREA OBJECT',
                labelKey: 'measure.kind.area_object',
                icon: 'area',
                command: 'AREA OBJECT',
              },
            ],
          },
          // ID (2026-10-26) is the point-coordinate readout — AutoCAD's Measure
          // panel keeps it in the same group as Distance, so it is the fifth
          // measurement button here. It stays `large` like its four neighbours
          // (this panel has no small-entry row) and carries no variant popover:
          // there is no options sub-prompt. Its tool chip is `tool.measure.id`.
          { id: 'ID', labelKey: 'measure.kind.id', icon: 'point', command: 'ID', large: true },
        ],
      },
    ],
  },
  {
    id: 'annotate',
    labelKey: 'ribbon.annotate',
    panels: [
      {
        id: 'annotation',
        titleKey: 'ribbon.panel.annotation',
        items: [
          // Dimension creation leads the annotate panel, the way AutoCAD's
          // Annotate tab puts its Dimensions panel first. The linear/aligned pair
          // are three-pick captures (two extension-line origins, then the
          // dimension-line location); the radial pair are two-pick captures (an
          // arc/circle entity pick, then the dimension-line location); the centre
          // mark is a single arc/circle pick producing plain geometry; the angular
          // dimension is a two-line capture (two straight-line entity picks, then
          // the arc location that selects which angle is measured) and the
          // ordinate dimension is a two-point capture (a feature point, then the
          // leader endpoint).
          // The whole family stays together in one fixed order — linear, aligned,
          // radius, diameter, centre mark, angular, ordinate — so the panel never
          // reorders between sessions and no already-shipped button moves: the
          // angular/ordinate pair is appended in AutoCAD's own angular-before-
          // ordinate sequence instead of slotting between aligned and radius.
          //
          // DIMANGULAR is offered here as the **two-line** form only. The
          // three-point form exists at command level but has no interactive
          // capture, so no part of this button implies a vertex-first workflow.
          //
          // Each command carries its own glyph in the shared 24px stroke language
          // (`dimlinear`/`dimaligned`/`dimradius`/`dimdiameter`/`dimcenter`/
          // `dimangular`/`dimordinate`/`dimbaseline`/`dimcontinue`), so the family
          // reads as one set: the radial pair draws its arc/circle target with the
          // linear pair's dimension-line + arrowhead vocabulary, the centre mark
          // shows the crossed mark at the circle's centre, the angular glyph adds
          // dimension arrowheads to the measure-angle rays-plus-arc, and the
          // ordinate glyph is a feature point whose leader ends in a dimension
          // tick.
          { id: 'DIMLINEAR', labelKey: 'draw.kind.dimlinear', icon: 'dimlinear', command: 'DIMLINEAR', large: true },
          { id: 'DIMALIGNED', labelKey: 'draw.kind.dimaligned', icon: 'dimaligned', command: 'DIMALIGNED', large: true },
          { id: 'DIMRADIUS', labelKey: 'draw.kind.dimradius', icon: 'dimradius', command: 'DIMRADIUS', large: true },
          { id: 'DIMDIAMETER', labelKey: 'draw.kind.dimdiameter', icon: 'dimdiameter', command: 'DIMDIAMETER', large: true },
          { id: 'DIMCENTER', labelKey: 'draw.kind.dimcenter', icon: 'dimcenter', command: 'DIMCENTER', large: true },
          { id: 'DIMANGULAR', labelKey: 'draw.kind.dimangular', icon: 'dimangular', command: 'DIMANGULAR', large: true },
          { id: 'DIMORDINATE', labelKey: 'draw.kind.dimordinate', icon: 'dimordinate', command: 'DIMORDINATE', large: true },
          // Baseline/continue (ADR 0017 decision C, P5f) close the family. Both
          // start by picking an **existing** linear dimension as their base, then
          // take any number of points and commit with Enter, so they are appended
          // after the seven creation commands (no shipped button moves) and last
          // in the family: they consume a dimension that already exists instead of
          // creating the first one.
          { id: 'DIMBASELINE', labelKey: 'draw.kind.dimbaseline', icon: 'dimbaseline', command: 'DIMBASELINE', large: true },
          { id: 'DIMCONTINUE', labelKey: 'draw.kind.dimcontinue', icon: 'dimcontinue', command: 'DIMCONTINUE', large: true },
          { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', action: 'properties', large: true },
          { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', command: 'LAYER', large: true },
          { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', command: 'SELECTALL' },
        ],
      },
    ],
  },
  {
    id: 'view',
    labelKey: 'ribbon.view',
    panels: [
      // The 3D affordances lead the View tab the way AutoCAD's View tab leads
      // with Visualize: the mode toggle, the projection toggle and the orbit
      // hint. `SWITCH2D3D` and `PROJECTION` are payload-free router commands, so
      // they travel on `cad_command`; both are document-scoped and read as
      // disabled on the start page.
      {
        id: 'view3d',
        titleKey: 'view3d.title',
        items: [
          {
            id: 'SWITCH2D3D',
            labelKey: 'ribbon.command.view_switch2d3d',
            icon: 'view',
            command: 'SWITCH2D3D',
            large: true,
          },
          { id: 'PROJECTION', labelKey: 'view.projection', icon: 'axes', command: 'PROJECTION' },
          // Orbit stays a **gesture**: in a 3D view a primary canvas drag emits
          // `CommandId::Orbit` with the yaw/pitch of the pointer motion, and
          // `cad_command` refuses a payload-free `ORBIT`. The affordance is
          // visible but disabled with `view.orbit.gesture` (through
          // `unavailableReasonKey`), so no click can fabricate a camera move.
          { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', command: 'ORBIT' },
        ],
      },
      // Standard views carry a payload (`CommandPayload::StandardView`), so they
      // are **not** `command` items: each names its host machine key in
      // `standardView` and the renderer sends it on the dedicated typed IPC
      // `cad_standard_view`. `top` leaves 3D for the 2D plan; the other six are
      // framed 3D perspectives. The list is data (`STANDARD_VIEWS`) so the model
      // and the tests cannot drift apart.
      //
      // `rows: 2` is the layout contract, not a decoration: the small-item grid
      // flows column-major, so two rows put top/bottom, front/back and
      // left/right in the same column as neighbours (they share an edge) and
      // leave isometric alone in the last column. With the three-row default
      // every pair was split across two columns — top above back, front beside
      // left — which reads as an arbitrary list rather than three pairs.
      {
        id: 'standard',
        titleKey: 'view.standard_label',
        rows: 2,
        items: STANDARD_VIEWS.map(entry => ({ ...entry })),
      },
      {
        id: 'navigate',
        titleKey: 'ribbon.panel.navigate',
        launcher: true,
        items: [
          { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', command: 'ZOOMIN' },
          { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', command: 'ZOOMOUT' },
          { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', command: 'PAN' },
          { id: 'FIT', labelKey: 'toolbar.fit', icon: 'fit', command: 'FIT' },
          // RESETVIEW closes the navigate cluster: it is the payload-free
          // `CommandId::ResetView` selector, so it travels on `cad_command` like
          // its neighbours and is enabled by router admission alone.
          { id: 'RESETVIEW', labelKey: 'view.reset', icon: 'view', command: 'RESETVIEW' },
        ],
      },
      {
        id: 'display',
        titleKey: 'ribbon.panel.display',
        items: [
          { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', command: 'GRID' },
          { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', command: 'SNAP' },
          { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', command: 'ORTHO' },
        ],
      },
      {
        id: 'palettes',
        titleKey: 'ribbon.panel.utilities',
        items: [
          { id: 'diagnostics', labelKey: 'diagnostics.title', icon: 'diagnostics', action: 'diagnostics', large: true },
          { id: 'clean', labelKey: 'status.clean_screen', icon: 'cleanScreen', action: 'cleanScreen' },
        ],
      },
    ],
  },
]);

export function ribbonTab(id: string): RibbonTab {
  return RIBBON_TABS.find(tab => tab.id === id) ?? RIBBON_TABS[1];
}

/**
 * Cell coordinates for a column-major panel grid with `rows` rows.
 *
 * This mirrors the layout `.ribbon-panel__group.is-small` actually produces
 * (`display: grid; grid-auto-flow: column; grid-template-rows: repeat(rows, …)`),
 * so a test can assert what a reader will see from the model alone: item 1 and
 * item 2 are neighbours *only* because they share a column.
 */
export function columnMajorCells<T>(items: readonly T[], rows: number): RibbonCell<T>[] {
  const height = Math.max(1, Math.trunc(rows) || 1);
  return items.map((item, index) => ({ item, column: Math.floor(index / height), row: index % height }));
}

/**
 * Whether every pair in `pairs` lands in two cells that share an edge
 * (vertically adjacent in one column, or horizontally adjacent in one row).
 *
 * `key` extracts the comparison token from an item, so the same helper works for
 * `standardView` names and for any other pairwise grouping.
 */
export function pairsShareEdge<T extends { readonly id: string }>(
  items: readonly T[],
  rows: number,
  pairs: readonly (readonly [string, string])[],
  key: (item: T) => string = item => item.id,
): boolean {
  const cells = new Map(columnMajorCells(items, rows).map(cell => [key(cell.item), cell]));
  return pairs.every(([a, b]) => {
    const first = cells.get(a);
    const second = cells.get(b);
    if (!first || !second) return false;
    if (first.column === second.column) return Math.abs(first.row - second.row) === 1;
    return first.row === second.row && Math.abs(first.column - second.column) === 1;
  });
}

/** All command ids referenced by the ribbon (used to keep the catalog honest). */
export function ribbonCommandIds(): string[] {
  const ids = new Set<string>();
  for (const tab of RIBBON_TABS) {
    for (const panel of tab.panels) {
      for (const item of panel.items) {
        if (typeof item.command === 'string') ids.add(item.command);
        for (const variant of item.variants ?? []) if (typeof variant.command === 'string') ids.add(variant.command);
      }
    }
  }
  return [...ids];
}
