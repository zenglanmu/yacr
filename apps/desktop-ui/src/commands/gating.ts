// The chrome's declarative admission tables, split verbatim out of `commands.ts`
// (which owns the functions that read them) so neither module approaches the
// 1000-line code gate. Plain sibling module, not a barrel: importers name it
// directly.
//
// Nothing here decides anything by itself — these are the frozen data rows the
// admission functions consult: the typed-argument grammar, the host capability
// gates, the gesture-only and document-scoped sets, the selection preconditions
// and the selectors the chrome answers without the router. Empty or wrong data
// here can only ever refuse, never silently admit.
import {
  ABOUT_COMMAND,
  CLEANSCREENOFF_COMMAND,
  CLEANSCREENON_COMMAND,
  LAYERCLOSE_COMMAND,
  PROPERTIESCLOSE_COMMAND,
} from './chrome-commands.ts';

/** The `cad_status` slice the admission rules read (all fields are the host's). */
export interface CommandStatusView {
  loading?: boolean;
  documentLabel?: string | null;
  canOpen?: boolean;
  canNew?: boolean;
  canSave?: boolean;
  canPlot?: boolean;
  canFit?: boolean;
  entityCount?: number | null;
  /** The `cad_status.selection` slice the selection rules count (`{count}`). */
  selection?: { readonly count?: unknown } | null;
}

/** A host capability boolean that gates a file-level selector. */
export type CapabilityGate = 'canNew' | 'canSave' | 'canPlot';

/** The parser for the one typed-argument form a selector accepts. */
export interface ArgumentSpec {
  readonly parse: (argument: string) => boolean;
}

/** Whether `text` is a plain positive decimal number (`2`, `0.5`, `1.25`). */
function isPositiveDecimal(text: string): boolean {
  if (!/^[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  const value = Number(text);
  return Number.isFinite(value) && value > 0;
}

/**
 * Whether `text` is a plain decimal number, sign and zero included (`3`, `2.5`,
 * `0`, `-5`).
 *
 * The point-style sysvars need the signed form where `SNAPUNIT` does not:
 * `PDMODE 0` is the DWG default (dot) and `PDSIZE 0` is the "size from the
 * viewport" default, so a positive-only gate would refuse the drawing's own
 * defaults; a negative `PDSIZE` is a percentage of the viewport height in the
 * DWG model, and the renderer reads the sign. The value is still a **single**
 * plain decimal (`5.`, `.5`, `1e3`, `+5` and `2 3` are not one) — the same
 * literal shape the other sysvar uses, so one grammar is typed either way.
 */
function isDecimalNumber(text: string): boolean {
  if (!/^-?[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  return Number.isFinite(Number(text));
}

/**
 * Whether `text` is a finite decimal with an optional explicit sign (`45`,
 * `22.5`, `0`, `-90`, `+30`).
 *
 * `TORIENT` accepts an absolute rotation in either direction, so the host's
 * grammar admits an explicit `+`; this mirrors it for local admission. It is
 * otherwise the same single plain decimal (no exponent, no bare `.`/`5.`, no
 * embedded spaces).
 */
function isSignedDecimal(text: string): boolean {
  if (!/^[+-]?[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  return Number.isFinite(Number(text));
}

/**
 * Whether `text` is a finite decimal with an optional sign that is **greater
 * than zero** (`2`, `0.5`, `+3`; never `0`, `-1`).
 *
 * `HATCHEDIT SCALE` needs this shape: the host's grammar is the same plain
 * decimal plus the strict positivity rule, so a zero/negative factor is refused
 * locally instead of reaching the router.
 */
function isPositiveSignedDecimal(text: string): boolean {
  if (!/^\+?[0-9]+(\.[0-9]+)?$/.test(text)) return false;
  const value = Number(text);
  return Number.isFinite(value) && value > 0;
}

/**
 * The selectors that take a typed argument, with the **local syntax** predicate
 * for that argument.
 *
 * This is an admission aid only: the chrome decides "does this line look like a
 * command that could run" and the host stays the authority for semantics (a
 * `ZOOM 0` that slipped through would be refused by the router and recorded for
 * `cad_status.error`, never clamped). `ZOOM` mirrors the host grammar —
 * a positive decimal scale factor (`> 1` zooms in, `< 1` zooms out), `ALL` or
 * `EXTENTS` (the `FIT` framing), and the two word forms `WINDOW` / `PREVIOUS`
 * (below). `SNAPUNIT` is the session grid-snap spacing
 * sysvar and accepts the same positive-decimal shape (`25`, `0.5`, `2.5`).
 * `PDMODE`/`PDSIZE` (2026-10-26) are the drawing's POINT display-style sysvars:
 * one plain decimal each (`PDMODE 3`, `PDSIZE 2.5`), zero and negative included
 * because `PDMODE 0` (dot) and `PDSIZE 0` (viewport-relative) are the DWG
 * defaults and a negative `PDSIZE` is a viewport percentage. They change the
 * in-memory display style of the open drawing only — not undoable, and the
 * exporter writes neither — so nothing here may suggest persistence.
 * Every other selector takes no argument, so `LINE 2` and `SNAP 5` stay explicit
 * unsupported answers.
 */
export const ARGUMENT_COMMANDS: Readonly<Record<string, ArgumentSpec>> = Object.freeze({
  ZOOM: Object.freeze({
    /**
     * `ZOOM WINDOW` and `ZOOM PREVIOUS` (2026-10-28) are **argument forms of the
     * `ZOOM` selector**, not commands of their own: the host adds no `CommandId`
     * and advertises no extra name, so the word is grammar here exactly like
     * `ALL`/`EXTENTS` are, and the line still travels as one `cad_command`
     * (`"ZOOM WINDOW"`). They are word alternatives in the same grammar as the
     * decimal factor, so no word can ever be read as one (`isPositiveDecimal`
     * already refuses letters) and no new selector row is needed.
     *
     * This table is the *only* place the two words are listed for admission: the
     * command line admits them through the ordinary base-name path
     * (`splitCommandText` reads `ZOOM` + `WINDOW`), and the ribbon's
     * `shell.canCommand('ZOOM WINDOW')` is that same call — so the button is
     * enabled iff `ZOOM` itself is admitted, with no second name list and no
     * `HOST_NAMES` entry the host does not publish. Semantics stay the host's:
     * a 3D view refuses `WINDOW` by name, and `PREVIOUS` with no recorded step is
     * refused there too.
     */
    parse: (argument: string) => argument === 'ALL' || argument === 'EXTENTS'
      || argument === 'WINDOW' || argument === 'PREVIOUS'
      || isPositiveDecimal(argument),
  }),
  SNAPUNIT: Object.freeze({
    parse: isPositiveDecimal,
  }),
  // Point display style (`$PDMODE` / `$PDSIZE`): the same one-number grammar as
  // `SNAPUNIT`, widened to the signed/zero form the DWG values themselves use.
  // The range semantics (which mode means dot/cross/…) and whether a value is
  // expressible stay the host's call — a value refused here is refused before
  // IPC with the shared `command.unsupported` sentence, and a value admitted
  // here is still validated by the router.
  PDMODE: Object.freeze({
    parse: isDecimalNumber,
  }),
  PDSIZE: Object.freeze({
    parse: isDecimalNumber,
  }),
  /**
   * `SPACE` carries the target space as one argument, matching the host's own
   * grammar: `MODEL`, `#<numeric LayoutId>`, or a layout display name (the whole
   * remainder, spaces included). The name is resolved against the authoritative
   * drawing by the host — exact first, then a unique case-insensitive match —
   * and an unknown or ambiguous name is a named refusal. The chrome therefore
   * only requires a non-empty argument; it must not reject a real layout name.
   */
  SPACE: Object.freeze({
    parse: (argument: string) => argument.trim().length > 0,
  }),
  TORIENT: Object.freeze({
    parse: (argument: string) => isSignedDecimal(argument.trim()),
  }),
  TCASE: Object.freeze({
    parse: (argument: string) => ['UPPER', 'LOWER', 'TITLE', 'SENTENCE', 'TOGGLE', 'U', 'L', 'T', 'S', 'G']
      .includes(argument.trim().toUpperCase()),
  }),
  /**
   * `PEDIT <option>` (AutoCAD 2026) is the declared v1 option subset: `CLOSE`,
   * `OPEN` and `REVERSE`, plus their single letters `C`, `O`, `R`. The bare
   * `PEDIT` is admitted by the base name and refused by the host with
   * `pedit.needs_option`, because this build has no in-command option prompt —
   * so a missing argument is never read as a default option. The host validates
   * the same grammar and refuses every other AutoCAD option (`JOIN`, `WIDTH`,
   * `FIT`, `SPLINE`, …) by name.
   */
  PEDIT: Object.freeze({
    parse: (argument: string) => ['CLOSE', 'OPEN', 'REVERSE', 'C', 'O', 'R']
      .includes(argument.trim().toUpperCase()),
  }),
  /**
   * `HATCHEDIT <option>` (AutoCAD 2026) is the typed v1 hatch edit. The four
   * implemented forms are `ANGLE <degrees>` (one finite signed decimal),
   * `SCALE <factor>` (one finite decimal > 0), `SOLID` and `GRADIENT`. The bare
   * `HATCHEDIT` is admitted by the base name and refused by the host with
   * `hatchedit.needs_option`; every other AutoCAD dialog field (`Pattern`,
   * `Color`, `Properties`, `Draw order`, boundary add/remove, associativity) is
   * NOT IMPLEMENTED and is refused locally.
   */
  HATCHEDIT: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().toUpperCase().split(/\s+/).filter(part => part.length > 0);
      const [word, ...rest] = parts;
      if (word === 'SOLID' || word === 'GRADIENT') return rest.length === 0;
      if (word === 'ANGLE') return rest.length === 1 && isSignedDecimal(rest[0]);
      if (word === 'SCALE') return rest.length === 1 && isPositiveSignedDecimal(rest[0]);
      return false;
    },
  }),
  MEASUREGEOM: Object.freeze({
    parse: (argument: string) => ['DISTANCE', 'ANGLE', 'AREA', 'POLYLINE', 'RADIUS', 'VOLUME']
      .includes(argument.trim().toUpperCase()),
  }),
  JUSTIFYTEXT: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().toUpperCase().split(/\s+/).filter(part => part.length > 0);
      return parts.length === 2
        && ['LEFT', 'CENTER', 'RIGHT'].includes(parts[0])
        && ['BASELINE', 'BOTTOM', 'MIDDLE', 'TOP'].includes(parts[1]);
    },
  }),
  TJUST: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().toUpperCase().split(/\s+/).filter(part => part.length > 0);
      return parts.length === 2
        && ['LEFT', 'CENTER', 'RIGHT'].includes(parts[0])
        && ['BASELINE', 'BOTTOM', 'MIDDLE', 'TOP'].includes(parts[1]);
    },
  }),
  TSCALE: Object.freeze({
    parse: isPositiveDecimal,
  }),
  TEXTALIGN: Object.freeze({
    parse: (argument: string) => ['LEFT', 'RIGHT', 'CENTER', 'TOP', 'MIDDLE', 'BOTTOM']
      .includes(argument.trim().toUpperCase()),
  }),
  TCOUNT: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().split(/\s+/).filter(part => part.length > 0);
      if (parts.length < 2 || parts.length > 4) return false;
      if (!/^[+-]?\d+$/.test(parts[0]) || !/^[+-]?\d+$/.test(parts[1])) return false;
      return Number.parseInt(parts[1], 10) > 0;
    },
  }),
  TCIRCLE: Object.freeze({
    parse: (argument: string) => ['CIRCLE', 'RECTANGLE']
      .includes(argument.trim().toUpperCase()),
  }),
  LTSCALE: Object.freeze({
    parse: isPositiveDecimal,
  }),
  LAYERPMODE: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF'].includes(argument.trim().toUpperCase()),
  }),
  LAYERSTATE: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().split(/\s+/).filter(part => part.length > 0);
      const [word, ...rest] = parts;
      const keyword = (word ?? '').toUpperCase();
      if (keyword === 'LIST') return rest.length === 0;
      if (keyword === 'SAVE' || keyword === 'RESTORE' || keyword === 'DELETE') {
        return rest.length > 0;
      }
      return false;
    },
  }),
  LAYWALK: Object.freeze({
    parse: (argument: string) => {
      const names = argument.split(',').map(name => name.trim());
      return names.length > 0 && names.every(name => name.length > 0);
    },
  }),
  QSELECT: Object.freeze({
    parse: (argument: string) => isPropertyFilter(argument),
  }),
  FILTER: Object.freeze({
    parse: (argument: string) => isPropertyFilter(argument),
  }),
  GETSEL: Object.freeze({
    parse: (argument: string) => isPropertyFilter(argument, ['LAYER', 'TYPE']),
  }),
  FIND: Object.freeze({
    parse: isFindArgument,
  }),
  // `-FIND` is the report-only selector: a `=>` replacement is refused locally
  // as well as by the host, so the button/line never promises a write.
  '-FIND': Object.freeze({
    parse: (argument: string) => isFindArgument(argument) && !argument.includes('=>'),
  }),
  PURGE: Object.freeze({
    parse: (argument: string) => ['LAYER', 'BLOCK', 'STYLE', 'LINETYPE']
      .includes(argument.trim().toUpperCase()),
  }),
  RENAME: Object.freeze({
    parse: isRenameArgument,
  }),
  SETVAR: Object.freeze({
    parse: (argument: string) => /^[A-Za-z][A-Za-z0-9_]*(?:[ \t]+.+)?$/.test(argument.trim()),
  }),
  MREDO: Object.freeze({
    parse: (argument: string) => /^[0-9]+$/.test(argument.trim())
      && Number.parseInt(argument.trim(), 10) > 0,
  }),
  MULTIPLE: Object.freeze({
    parse: (argument: string) => argument.trim().length > 0,
  }),
  DONUT: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 4 && parts.every(isSignedDecimal);
    },
  }),
  SOLID: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return (parts.length === 3 || parts.length === 4) && parts.every(isCoordinatePair);
    },
  }),
  HELIX: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 5 && parts.every(isSignedDecimal);
    },
  }),
  MLINE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 4 && isPositiveDecimal(parts[0])
        && isSignedDecimal(parts[1]) && isCoordinatePair(parts[2])
        && isCoordinatePair(parts[3]);
    },
  }),
  MLSTYLE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const [word, ...rest] = parts;
      if (word.toUpperCase() === 'LIST') return rest.length === 0;
      return word.toUpperCase() === 'SET' && rest.length === 2
        && isPositiveDecimal(rest[1]);
    },
  }),
  TREX: Object.freeze({
    parse: (argument: string) => ['TRIM', 'EXTEND'].includes(argument.trim().toUpperCase()),
  }),
  MPEDIT: Object.freeze({
    parse: (argument: string) => ['CLOSE', 'OPEN', 'REVERSE', 'C', 'O', 'R']
      .includes(argument.trim().toUpperCase()),
  }),
  DRAWORDER: Object.freeze({
    parse: (argument: string) => ['FRONT', 'BACK', 'ABOVE', 'BELOW']
      .includes(argument.trim().toUpperCase()),
  }),
  VIEW: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().split(/\s+/).filter(part => part.length > 0);
      const [word, ...rest] = parts;
      const keyword = (word ?? '').toUpperCase();
      if (keyword === 'LIST') return rest.length === 0;
      if (keyword === 'SAVE' || keyword === 'RESTORE' || keyword === 'DELETE') {
        return rest.length > 0;
      }
      return false;
    },
  }),
  VIEWGO: Object.freeze({
    parse: (argument: string) => argument.trim().length > 0,
  }),
  VPOINT: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().split(/\s+/).filter(part => part.length > 0);
      if (parts.length === 1) {
        return ['TOP', 'BOTTOM', 'FRONT', 'BACK', 'LEFT', 'RIGHT']
          .includes(parts[0].toUpperCase());
      }
      return parts.length === 3 && parts.every(part => isSignedDecimal(part));
    },
  }),

  // The 2026-11-07 extras batch. `-PAN` and `-LINETYPE` share their canonical
  // grammar so a typed dash form is not refused locally.
  PAN: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return (parts.length === 2 || parts.length === 4) && parts.every(isSignedDecimal);
    },
  }),
  '-PAN': Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return (parts.length === 2 || parts.length === 4) && parts.every(isSignedDecimal);
    },
  }),
  TRACE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 5 && parts.every(isSignedDecimal) && isPositiveDecimal(parts[4]);
    },
  }),
  MOCORO: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const word = (parts[0] ?? '').toUpperCase();
      if (word === 'MOVE' || word === 'COPY') {
        return parts.length === 3 && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2]);
      }
      if (word === 'ROTATE') {
        return parts.length === 4
          && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2])
          && isSignedDecimal(parts[3]);
      }
      return word === 'SCALE' && parts.length === 4
        && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2])
        && isPositiveSignedDecimal(parts[3]);
    },
  }),
  VIEWRES: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length !== 1 || !/^[0-9]+$/.test(parts[0])) return false;
      const percent = Number(parts[0]);
      return percent >= 1 && percent <= 1000;
    },
  }),
  VSCURRENT: Object.freeze({
    // The host names every unsupported style, so the local gate admits the
    // shape (one word) and lets that refusal speak.
    parse: (argument: string) => tokens(argument).length === 1,
  }),
  LINETYPE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const word = (parts[0] ?? '').toUpperCase();
      if (word === 'LIST') return parts.length === 1;
      return word === 'SET' && parts.length >= 2;
    },
  }),
  '-LINETYPE': Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const word = (parts[0] ?? '').toUpperCase();
      if (word === 'LIST') return parts.length === 1;
      return word === 'SET' && parts.length >= 2;
    },
  }),

  LAYOUT: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const [option, ...rest] = parts;
      if (option === undefined) return false;
      const upper = option.toUpperCase();
      if (upper === 'LIST') return rest.length === 0;
      if (upper === 'NEW' || upper === 'DELETE' || upper === 'SET') return rest.length > 0;
      return upper === 'RENAME' && rest.length >= 2;
    },
  }),
  LAYOUTMERGE: Object.freeze({
    parse: (argument: string) => tokens(argument).length >= 2,
  }),
  LAYOUTWIZARD: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length === 1) return true;
      return parts.length === 4 && isPositiveDecimal(parts[2]) && isPositiveDecimal(parts[3]);
    },
  }),
  VPORTS: Object.freeze({
    parse: parseVportsGrid,
  }),
  VPCLIP: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      // The host names a non-RECT boundary (`vpclip.polygonal_not_implemented`),
      // so the chrome admits the shape and lets that refusal speak.
      return parts.length === 5 && parts.slice(1).every(isSignedDecimal);
    },
  }),
  VPSCALE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length !== 1) return false;
      if (!parts[0].includes(':')) return true;
      const ratio = parts[0].split(':');
      return ratio.length === 2 && ratio.every(isSignedDecimal);
    },
  }),
  LAYVPI: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.split(',').map(part => part.trim());
      return parts.length > 0 && parts.every(part => part.length > 0);
    },
  }),
  // `-VPORTS` is an alias spelling; it shares the canonical grid grammar so a
  // typed `-VPORTS 2 3` is not refused locally before the host can normalize it.
  '-VPORTS': Object.freeze({
    parse: parseVportsGrid,
  }),

  DIMSTYLE: Object.freeze({
    parse: isDimStyleArgument,
  }),
  '-DIMSTYLE': Object.freeze({
    parse: isDimStyleArgument,
  }),
  DIMTEDIT: Object.freeze({
    parse: isDimTextEditArgument,
  }),
  DIMEDIT: Object.freeze({
    parse: isDimEditArgument,
  }),
  DIMOVERRIDE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length >= 2 && /^[A-Za-z][A-Za-z0-9_]*$/.test(parts[0]);
    },
  }),
  DIMJOGLINE: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const word = (parts[0] ?? '').toUpperCase();
      if (word === 'REMOVE') return parts.length === 1;
      return word === 'ADD' && parts.length === 3
        && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2]);
    },
  }),

});

/** `DIMSTYLE`/`-DIMSTYLE`: `LIST`, `CURRENT` or `SET <name>` (name may have spaces). */
function isDimStyleArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'LIST' || word === 'CURRENT') return parts.length === 1;
  return word === 'SET' && parts.length >= 2;
}

/** `DIMTEDIT`: `MOVE <x> <y>`, `ROTATE <degrees>` or `RESET`. */
function isDimTextEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'RESET') return parts.length === 1;
  if (word === 'MOVE') {
    return parts.length === 3 && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2]);
  }
  if (word === 'ROTATE') return parts.length === 2 && isSignedDecimal(parts[1]);
  return false;
}

/** `DIMEDIT`: `TEXT <override>` (non-empty remainder) or `RESET`. */
function isDimEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'RESET') return parts.length === 1;
  return word === 'TEXT' && parts.length >= 2;
}

/** Split a typed argument into whitespace-separated tokens. */
function tokens(argument: string): readonly string[] {
  return argument.trim().split(/\s+/).filter(part => part.length > 0);
}

/** `VPORTS <rows> <cols>` / `-VPORTS`: two positive whole numbers. */
function parseVportsGrid(argument: string): boolean {
  const parts = tokens(argument);
  return parts.length === 2
    && /^[0-9]+$/.test(parts[0]) && /^[0-9]+$/.test(parts[1])
    && Number(parts[0]) >= 1 && Number(parts[1]) >= 1;
}

/** Whether `text` is one `x,y` pair of finite signed decimals. */
function isCoordinatePair(text: string): boolean {
  const parts = text.split(',');
  return parts.length === 2 && parts.every(isSignedDecimal);
}


/** The four property words `QSELECT`/`FILTER` accept. */
const FILTER_PROPERTIES: readonly string[] = Object.freeze(['TYPE', 'LAYER', 'COLOR', 'LINETYPE']);

/**
 * `<PROPERTY> <value>` with a non-empty trimmed value (spaces allowed).
 *
 * Mirrors the host grammar: the property word is one of `FILTER_PROPERTIES` and
 * the value is the whole remainder, so a layer/linetype name with a space is not
 * rejected locally. `allowed` narrows the set for `GETSEL` (`LAYER`/`TYPE`).
 */
function isPropertyFilter(argument: string, allowed: readonly string[] = FILTER_PROPERTIES): boolean {
  const match = /^([^ \t]+)[ \t]+(.+)$/.exec(argument.trim());
  return match !== null && allowed.includes(match[1].toUpperCase()) && match[2].trim().length > 0;
}

/**
 * `<needle>` or `<needle> => <replacement>` with a non-empty needle.
 *
 * `-FIND` is report-only, so its spec below additionally refuses the arrow. The
 * replacement may be empty (an explicit delete) — the marker is what makes it
 * intentional.
 */
function isFindArgument(argument: string): boolean {
  const trimmed = argument.trim();
  if (!trimmed) return false;
  const marker = trimmed.indexOf('=>');
  return marker === -1 ? true : trimmed.slice(0, marker).trim().length > 0;
}

/**
 * `RENAME <kind> <old> <new>` with quote-aware tokens.
 *
 * The kind is one of the four named-object words; names containing spaces must be
 * double-quoted so the old/new boundary is unambiguous. This mirrors the host
 * parser; the host still owns the actual name grammar.
 */
function isRenameArgument(argument: string): boolean {
  const tokens = splitQuotedTokens(argument);
  if (tokens === null || tokens.length !== 3) return false;
  if (!['LAYER', 'BLOCK', 'STYLE', 'LINETYPE'].includes(tokens[0].toUpperCase())) return false;
  return tokens[1].length > 0 && tokens[2].length > 0;
}

/**
 * Split whitespace-separated tokens while keeping double-quoted runs together.
 * Returns `null` for an unbalanced quote. Unquoted empty runs are ignored.
 */
function splitQuotedTokens(text: string): string[] | null {
  const tokens: string[] = [];
  let current = '';
  let quoted = false;
  let started = false;
  for (const ch of text.trim()) {
    if (ch === '"') {
      quoted = !quoted;
      started = true;
    } else if (/\s/.test(ch) && !quoted) {
      if (started || current.length > 0) {
        tokens.push(current);
        current = '';
        started = false;
      }
    } else {
      current += ch;
      started = true;
    }
  }
  if (quoted) return null;
  if (started || current.length > 0) tokens.push(current);
  return tokens;
}

/**
 * Host capability gates for the file-level selectors.
 *
 * `NEW`/`SAVE`/`PLOT` and their router aliases are real router commands, not
 * "host-owned" names to be hidden behind a local exclusion list: the host
 * publishes whether it can service them right now in `cad_status`
 * (`canNew`/`canSave`/`canPlot`) and those booleans decide admission. This table
 * maps each name onto the field that gates it; a name absent here has no
 * capability gate. `canNew` is about creating a blank drawing (no document
 * needed); `canSave`/`canPlot` additionally require an open document.
 */
export const CAPABILITY_GATES: Readonly<Record<string, CapabilityGate>> = Object.freeze({
  NEW: 'canNew',
  SAVE: 'canSave',
  SAVEAS: 'canSave',
  QSAVE: 'canSave',
  PLOT: 'canPlot',
  EXPORT: 'canPlot',
  PRINT: 'canPlot',
});

/**
 * Commands the Rust router accepts but a **click cannot perform** because they
 * are canvas drag gestures: `ORBIT` is the 3D primary drag, and its `yaw`/
 * `pitch` come from pointer motion, not from a button payload. Inventing an
 * orbit here would fabricate a camera move, so the affordance stays visible but
 * disabled with a gesture reason that tells the user how to orbit. This is an
 * explicit disable, never a local enable.
 */
export const GESTURE_ONLY_COMMANDS: readonly string[] = Object.freeze(['ORBIT']);

/**
 * Router-admitted selectors that only mean something with a drawing open:
 * `SWITCH2D3D` frames the document's 3D bounds and `PROJECTION` flips the
 * projection of that same camera, so on the start page there is nothing to
 * frame. `MODEL` returns to the Model tab of the open drawing, and `PLAN` returns
 * the open drawing's view to the 2D plan (there is nothing to view without one).
 * `AUDIT` validates the open drawing's stored entities; with no drawing there is
 * nothing to check.
 * They are disabled there with the catalog's own "no drawing is open" sentence
 * instead of a misleading "not supported yet".
 *
 * `MATCHPROP` needs a drawing *and* a selection; it is deliberately **not** in
 * this list — a document is not its only precondition, and the set means exactly
 * "a drawing is the whole requirement" (see `SELECTION_REQUIREMENTS`, which
 * answers the no-document case for selection-gated commands itself).
 */
export const DOCUMENT_SCOPED_COMMANDS: readonly string[] = Object.freeze([
  'SWITCH2D3D', 'PROJECTION', 'MODEL', 'PLAN', 'AUDIT', 'STATUS', 'LAYTHW', 'HATCHTOBACK',
  'LAYERSTATE', 'LTSCALE', 'LAYWALK',
  // The 2026-11-07 selection/filter/find/visibility batch queries or hides the
  // open drawing's entities, so the start page disables all ten with the
  // catalog's own "no drawing is open" sentence.
  'QSELECT', 'FILTER', 'GETSEL', 'SSX', 'SELECTCOUNT', '-FIND', 'FIND',
  'HIDEOBJECTS', 'ISOLATEOBJECTS', 'UNISOLATEOBJECTS',
  // The 2026-11-07 utility batch reports or edits the open document (or its
  // history), so the start page disables all eight with the catalog's own
  // "no drawing is open" sentence.
  'PURGE', 'RENAME', 'SETVAR', 'TIME', 'DBLIST', 'TREESTAT', 'MREDO', 'MULTIPLE',
  // The 2026-11-07 view batch refreshes, regenerates or restores a view of the
  // open drawing, so the start page disables all of it with the catalog's own
  // "no drawing is open" sentence.
  'REDRAW', 'REDRAWALL', 'REGEN', 'REGENALL', 'VIEW', 'VIEWGO', 'VIEWBACK', 'VIEWFORWARD',
  'VPOINT',
  // The 2026-11-07 layout/viewport batch reads or writes the open drawing's
  // layout table / session space, so the start page disables all nine.
  'LAYOUT', 'LAYOUTMERGE', 'LAYOUTWIZARD', 'VPORTS', 'VPMAX', 'VPMIN', 'VPCLIP',
  'VPSCALE', 'LAYVPI',

  // The 2026-11-07 dimension batch: the style reports and the two no-op reports
  // read the open drawing, so the start page disables them with the same
  // "no drawing is open" sentence. The edit commands are selection-gated and
  // answer the no-document case through their own hint row.
  'DIMSTYLE', '-DIMSTYLE', 'DIMREGEN', 'DIMDISASSOCIATE',

  // The 2026-11-07 extras batch reads or writes the open drawing / its session
  // view tokens. `REGEN3`, `-PAN` and `-LINETYPE` are alias spellings.
  'SKETCH', 'TRACE', 'MOCORO', 'MSTRETCH', 'VIEWRES', 'VSCURRENT', 'LINETYPE',
  '-LINETYPE', '-PAN', 'REGEN3',

]);

/** One command's precondition on the **current selection**. */
export interface SelectionRequirement {
  /** Minimum selected objects that can still produce a real result. */
  readonly count: number;
  /**
   * The catalog sentence that words the requirement. It is the desktop's own
   * copy of the host rule (`matchprop.needs_two`), so the disabled ribbon hint
   * and the command line's refusal say the same thing — and it is the *only*
   * place the source→targets rule is spelled out for the reader.
   */
  readonly reasonKey: string;
}

/**
 * Commands whose bare name acts on the **current selection**, with the smallest
 * selection that is not a guaranteed refusal.
 *
 * `MATCHPROP` is the selection-based property painter: the first selected entity
 * is the **source**, every other selected entity is a **target**, so one object
 * or none is never a partial success — the host refuses it outright
 * (`matchprop.needs_two`). Offering a live button in that state would be an
 * affordance that can only fail, so the same table disables it with a sentence
 * that says what is missing (`draw.kind.matchprop.hint`) *and* stops the command
 * line from sending a line that cannot succeed.
 *
 * Only *count* preconditions belong here. A command that merely needs "something"
 * selected (ERASE, MOVE, COPY) keeps the neighbours' contract: it stays enabled
 * and the host answers with an explicit refusal, because its own copy
 * (`draw.error.selection_required`) already says so at run time.
 *
 * `JOIN` (2026-10-26) is the second real count precondition: it welds the
 * selection into a single entity (one LINE or one ARC), so one object or none can
 * never be a partial success and the host refuses it outright. Its sentence is the
 * catalog's own `draw.kind.join.hint`, which is also what the disabled ribbon
 * button shows, so the button and the refusal word the same rule once.
 *
 * `LAYISO` needs at least one object — isolating "nothing" would leave every
 * layer hidden and no way back from the canvas — and it reuses the catalog's
 * existing "Select objects first" line (`draw.error.selection_required`): that is
 * already the sentence the host refuses a bare isolate with, so no second copy of
 * the same fact is invented. `LAYUNISO` takes **no** selection (`count: 0`): its
 * entry exists for the "no drawing is open" answer only — the requirement can
 * never be unmet, because nothing is what it restores.
 *
 * `LAYOFF`/`LAYON` (2026-10-26) follow the isolate pair exactly. `LAYOFF` turns
 * **off** every layer the current selection sits on (the current layer excepted),
 * so with nothing selected there is no layer to turn off — the same one-object
 * minimum as `LAYISO`, worded with the same catalog sentence the host refuses a
 * bare `LAYOFF` with (`draw.error.selection_required`). `LAYON` turns every layer
 * back on and takes **no** selection (`count: 0`), so its entry is the
 * "no drawing is open" answer only, exactly like `LAYUNISO`'s.
 */
export const SELECTION_REQUIREMENTS: Readonly<Record<string, SelectionRequirement>> = Object.freeze({
  MATCHPROP: Object.freeze({ count: 2, reasonKey: 'draw.kind.matchprop.hint' }),
  JOIN: Object.freeze({ count: 2, reasonKey: 'draw.kind.join.hint' }),
  // SELECTSIMILAR (AutoCAD 2026) grows the selection from its **geometry kind**,
  // so there must already be one object to call the source. The host refuses an
  // empty selection by name (`selectsimilar.selection_required`); this row makes
  // the disabled button and the command line say the same thing, and it is also
  // the "no drawing is open" answer. Attribute matching is NOT implemented in
  // v1, so the hint must not promise one.
  SELECTSIMILAR: Object.freeze({ count: 1, reasonKey: 'draw.kind.selectsimilar.hint' }),
  // CHAMFER POLYLINE / FILLET POLYLINE (2026-10-27) are the Polyline modes of the
  // two corner commands: each rewrites **exactly one** preselected open polyline
  // in place (`CommandPayload::None`, so nothing is picked interactively). The
  // table states a *minimum* of one — the host is what refuses a selection of two
  // or a non-polyline, by name — and the sentence is this command's own hint, so
  // the disabled button and the refusal describe the same rule once. Zero
  // selected objects can never be a partial success, which is the whole reason
  // the row exists instead of leaving the host to answer at run time.
  'CHAMFER POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.chamfer_polyline.hint' }),
  'FILLET POLYLINE': Object.freeze({ count: 1, reasonKey: 'draw.kind.fillet_polyline.hint' }),
  // REVCLOUD (AutoCAD 2026) Object mode v1 rewrites **exactly one** preselected
  // closed straight-segment polyline in place (`CommandPayload::None`, so nothing
  // is picked interactively). The table states a minimum of one — the host is
  // what refuses a selection of two or a non-polyline by name — and the sentence
  // is the command's own hint, so the disabled button and the refusal describe
  // the same rule once.
  REVCLOUD: Object.freeze({ count: 1, reasonKey: 'draw.kind.revcloud.hint' }),
  // REVERSE (AutoCAD 2026) reverses at least one selected object; the host
  // refuses an empty selection by name (`reverse.selection`). The table minimum
  // is one and the sentence is the command's own hint.
  REVERSE: Object.freeze({ count: 1, reasonKey: 'draw.kind.reverse.hint' }),
  // PEDIT (AutoCAD 2026) v1 edits **exactly one** preselected model-space
  // polyline (CLOSE / OPEN / REVERSE; no Select polyline prompt, no Multiple).
  // The table states a minimum of one — the host is what refuses a selection of
  // two or a non-polyline by name — and the sentence is the command's own hint,
  // so the disabled button/popover row and the refusal describe the same rule.
  PEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.pedit.hint' }),
  // OOPS (AutoCAD 2026) restores the last ERASE and never reads the selection,
  // so it has no selection precondition. The row exists for the "no drawing is
  // open" answer only, like LAYERP.
  OOPS: Object.freeze({ count: 0, reasonKey: 'draw.kind.oops.hint' }),
  // OVERKILL (AutoCAD 2026) can only remove a duplicate when there are at least
  // two objects to compare, so the local minimum is two. The host refuses an
  // empty/one-object selection by name (`overkill.selection`).
  OVERKILL: Object.freeze({ count: 2, reasonKey: 'draw.kind.overkill.hint' }),
  // HATCHGENERATEBOUNDARY (AutoCAD 2026) needs at least one selected hatch; the
  // host refuses an empty selection and any non-hatch by name
  // (`hatchgenerateboundary.selection` / `.not_hatch`). The table minimum is one
  // and the sentence is the command's own hint, so the disabled button and the
  // refusal describe the same rule once.
  HATCHGENERATEBOUNDARY: Object.freeze({ count: 1, reasonKey: 'draw.kind.hatchgenerateboundary.hint' }),
  // GRADIENT (AutoCAD 2026) fills the current selection, so at least one closed
  // boundary must be selected. The table is a *minimum*: the host refuses a
  // non-closed/non-+Z boundary by name. The sentence is the command's own hint.
  GRADIENT: Object.freeze({ count: 1, reasonKey: 'draw.kind.gradient.hint' }),
  // HATCHEDIT edits exactly one selected hatch; the host refuses other counts,
  // non-hatches and non-addressable refs by name. The table states the minimum
  // one and uses the command's own hint, so the disabled button and the refusal
  // describe the same rule.
  HATCHEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.hatchedit.hint' }),
  // BREAKATPOINT is an **interactive tool** (pick the object, then the break
  // point), not a pre-selection command, so it has no selection precondition.
  // The row exists for the "no drawing is open" answer only.
  BREAKATPOINT: Object.freeze({ count: 0, reasonKey: 'draw.kind.breakatpoint.hint' }),
  LAYISO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYUNISO: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  LAYOFF: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
  LAYON: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  // AREA OBJECT (2026-10-27) measures **exactly one** closed object, so its rule is
  // a count of one: `SelectionRequirement.count` is a *minimum* here (the table's
  // shape does not express an upper bound), which means a selection of two is let
  // through locally and refused by the host **by name** — the same division of
  // labour every other selection rule uses. The sentence is this command's own
  // `measure.kind.area_object.hint`, which states the one-object rule and that the
  // command does not pick objects interactively, so the refusal and the catalogue
  // row cannot describe two different behaviours.
  'AREA OBJECT': Object.freeze({ count: 1, reasonKey: 'measure.kind.area_object.hint' }),
  // LAYERP restores the previous layer-override state, so no selection is ever the
  // reason it cannot run. Like LAYON/LAYUNISO its entry exists for the "no drawing
  // is open" answer only.
  LAYERP: Object.freeze({ count: 0, reasonKey: 'draw.error.selection_required' }),
  // LIST reports the current selection; zero objects is `list.selection`.
  LIST: Object.freeze({ count: 1, reasonKey: 'draw.kind.list.hint' }),
  // MASSPROP (2026-11-07) measures the selected closed planar regions, so at
  // least one object is required. The table is a *minimum* (two objects are let
  // through locally and the host decides coplanarity/kind by name); the sentence
  // is the command's own hint, so the disabled button and the refusal agree.
  MASSPROP: Object.freeze({ count: 1, reasonKey: 'draw.kind.massprop.hint' }),
  // MEASUREGEOM itself takes no selection up front: its tool options capture
  // their own points and only RADIUS reads the selection, so pre-gating the
  // command would disable the tool entries with an unrelated sentence. The host
  // answers the RADIUS case by name (`measuregeom.radius.selection_empty`).
  // FS needs exactly one seed; two or more is refused by the host by name.
  FS: Object.freeze({ count: 1, reasonKey: 'draw.kind.fs.hint' }),
  // The layer-selection run is pre-selection based; LAYTHW needs no selection.
  LAYCUR: Object.freeze({ count: 1, reasonKey: 'draw.kind.laycur.hint' }),
  LAYMCUR: Object.freeze({ count: 1, reasonKey: 'draw.kind.laymcur.hint' }),
  LAYFRZ: Object.freeze({ count: 1, reasonKey: 'draw.kind.layfrz.hint' }),
  LAYTHW: Object.freeze({ count: 0, reasonKey: 'draw.kind.laythw.hint' }),
  LAYLCK: Object.freeze({ count: 1, reasonKey: 'draw.kind.laylck.hint' }),
  LAYULK: Object.freeze({ count: 1, reasonKey: 'draw.kind.layulk.hint' }),
  SETBYLAYER: Object.freeze({ count: 1, reasonKey: 'draw.kind.setbylayer.hint' }),
  // The 2026-11-07 merge/delete run is pre-selection based like the rest of the
  // layer run. LAYMCH needs a destination pick plus at least one object to move,
  // so its minimum is two; LAYMRG/LAYDEL need one object to name the source.
  LAYMCH: Object.freeze({ count: 2, reasonKey: 'draw.kind.laymch.hint' }),
  LAYMRG: Object.freeze({ count: 1, reasonKey: 'draw.kind.laymrg.hint' }),
  LAYDEL: Object.freeze({ count: 1, reasonKey: 'draw.kind.laydel.hint' }),
  // The text run is selection-based (one object minimum); DIMSPACE needs a base
  // dimension plus at least one target.
  TORIENT: Object.freeze({ count: 1, reasonKey: 'draw.kind.torient.hint' }),
  TCASE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcase.hint' }),
  TXT2MTXT: Object.freeze({ count: 1, reasonKey: 'draw.kind.txt2mtxt.hint' }),
  DIMSPACE: Object.freeze({ count: 2, reasonKey: 'draw.kind.dimspace.hint' }),
  // TEXTTOFRONT (AutoCAD 2026) reads the **current selection** (a declared
  // divergence from AutoCAD's Select objects prompt). With nothing selected the
  // host refuses it by name (`texttofront.selection`), so the table disables the
  // button/line with its own hint: one object is the minimum that can produce a
  // real result. The host independently skips non-text selections with
  // per-entity diagnostics, so the local rule is only a minimum.
  TEXTTOFRONT: Object.freeze({ count: 1, reasonKey: 'draw.kind.texttofront.hint' }),
  // The 2026-11-07 2D text batch. Every one reads the current selection; the
  // host independently refuses a non-text target (whole command or per-entity,
  // as documented) and the empty-selection case by name, so these rows are
  // minimums. TEXTALIGN needs **two** objects: aligning one to itself can never
  // produce a real result. TJUST shares JUSTIFYTEXT's rule (it is the same
  // `CommandId`).
  JUSTIFYTEXT: Object.freeze({ count: 1, reasonKey: 'draw.kind.justifytext.hint' }),
  TJUST: Object.freeze({ count: 1, reasonKey: 'draw.kind.justifytext.hint' }),
  TSCALE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tscale.hint' }),
  TEXTALIGN: Object.freeze({ count: 2, reasonKey: 'draw.kind.textalign.hint' }),
  TCOUNT: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcount.hint' }),
  TCIRCLE: Object.freeze({ count: 1, reasonKey: 'draw.kind.tcircle.hint' }),
  TEXTMASK: Object.freeze({ count: 1, reasonKey: 'draw.kind.textmask.hint' }),
  TEXTUNMASK: Object.freeze({ count: 1, reasonKey: 'draw.kind.textunmask.hint' }),
  // SSX reads the **first selected object** as the template and SELECTCOUNT does
  // the same, so both need one selected object; the host refuses the empty case
  // by name (`ssx.selection_required` / `selectcount.selection_required`).
  SSX: Object.freeze({ count: 1, reasonKey: 'draw.kind.ssx.hint' }),
  SELECTCOUNT: Object.freeze({ count: 1, reasonKey: 'draw.kind.selectcount.hint' }),
  // HIDEOBJECTS hides the selection; ISOLATEOBJECTS keeps it and hides the rest.
  // Both refuse an empty selection, so the row disables the button with its own
  // sentence. UNISOLATEOBJECTS takes no selection (it only clears the hidden
  // set), so its `count: 0` row exists for the no-drawing answer only.
  HIDEOBJECTS: Object.freeze({ count: 1, reasonKey: 'draw.kind.hideobjects.hint' }),
  ISOLATEOBJECTS: Object.freeze({ count: 1, reasonKey: 'draw.kind.isolateobjects.hint' }),
  UNISOLATEOBJECTS: Object.freeze({ count: 0, reasonKey: 'draw.kind.unisolateobjects.hint' }),
  // The 2026-11-07 selection batch. TREX needs a target **and** a boundary, so
  // two objects is the minimum; EXTRIM needs a boundary plus a target; MPEDIT
  // edits one or more polylines; DRAWORDER FRONT/BACK needs one, while
  // ABOVE/BELOW additionally require a reference object (the host refuses the
  // one-object form with `draworder.reference_required`, so the local minimum
  // stays one and the sentence names the rule).
  TREX: Object.freeze({ count: 2, reasonKey: 'draw.kind.trex.hint' }),
  EXTRIM: Object.freeze({ count: 2, reasonKey: 'draw.kind.extrim.hint' }),
  MPEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.mpedit.hint' }),
  DRAWORDER: Object.freeze({ count: 1, reasonKey: 'draw.kind.draworder.hint' }),
  // The 2026-11-07 dimension-editing batch. DIMTEDIT needs **exactly** one
  // dimension (the host refuses anything else); the rest need at least one.
  // DIMBREAK's declared input is one dimension plus crossing objects, so its
  // local minimum is two even though the host refuses the capability by name.
  DIMTEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimtedit.hint' }),
  DIMEDIT: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimedit.hint' }),
  DIMOVERRIDE: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimoverride.hint' }),
  QDIM: Object.freeze({ count: 1, reasonKey: 'draw.kind.qdim.hint' }),
  DIMJOGLINE: Object.freeze({ count: 1, reasonKey: 'draw.kind.dimjogline.hint' }),
  DIMBREAK: Object.freeze({ count: 2, reasonKey: 'draw.kind.dimbreak.hint' }),
  // MOCORO transforms the current selection in every mode; the host refuses an
  // empty one by name (`mocoro.selection_required`), and this row reuses the
  // shared "Select objects first" sentence. LINETYPE's `LIST` needs no
  // selection, so it is deliberately absent (its `SET` half is host-refused).
  MOCORO: Object.freeze({ count: 1, reasonKey: 'draw.error.selection_required' }),
});

/**
 * Selectors the **chrome** answers itself, before the router. They have a real
 * command-line entry (typing the word works) but no reason to reach a router
 * selector: `PROPERTIES` opens the properties window, `LAYER` opens the layers
 * palette, `FONTS` opens the read-only Font Manager, `RESOURCES` opens
 * the References panel, `ABOUT` opens the existing About dialog (the same
 * `requestOpenPopup('about')` path as the File tab's action) and
 * `CLEANSCREENON` sets the chrome's existing clean-screen state (the same state
 * the File tab's `cleanScreen` toggle flips; it sets, never toggles).
 * `CLEANSCREENOFF` clears that same state and is likewise an explicit set, never
 * a toggle. `PROPERTIESCLOSE` closes the properties window (an idempotent
 * success when it is already closed), so it is not gated by layout either.
 * `LAYERCLOSE` does the same for the layers palette: the palette's own close
 * control and the typed word close the one window, and a second close is still
 * a success, so no layout gate applies.
 *
 * They are deliberately admitted regardless of the loaded host vocabulary (and
 * of whether a drawing is open): they touch no CAD state, exactly like the
 * `OPTIONS` dialog the command line already intercepts locally. `LAYER` is not a
 * router name at all — the router knows the layer *operations*
 * (`TOGGLELAYER`/`SETLAYERS`/`RESTORELAYERS`/`SETACTIVELAYER`), which the palette
 * drives through its own dedicated commands (`cad_layers` and friends).
 * `RESOURCES` is a router name (it builds a host-side report), but the References
 * panel reads the references the status already carries, so the UI selector
 * opens that panel and sends nothing: no second source and no fabricated load.
 * `FONTS` owns its own report command (`cad_fonts`), which the panel reads
 * directly; the selector only opens the panel. `ABOUT` sends nothing at all: the
 * host-owned dialog already exists, and the selector only opens it.
 * `CLEANSCREENON` likewise sends nothing: it only sets the chrome's own
 * clean-screen state. `CLEANSCREENOFF` sends nothing either: it only clears that
 * same state. `PROPERTIESCLOSE` sends nothing: it only closes the chrome's own
 * properties window. `LAYERCLOSE` sends nothing either: it only closes the
 * chrome's own layers palette.
 */
export const LOCAL_CHROME_COMMANDS: readonly string[] = Object.freeze([
  'PROPERTIES', 'LAYER', 'FONTS', 'RESOURCES', ABOUT_COMMAND, CLEANSCREENON_COMMAND,
  CLEANSCREENOFF_COMMAND, PROPERTIESCLOSE_COMMAND, LAYERCLOSE_COMMAND,
]);
