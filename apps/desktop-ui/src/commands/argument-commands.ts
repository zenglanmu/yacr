// The typed-argument grammar `gating.ts` exposes as `ARGUMENT_COMMANDS`, split
// verbatim into a sibling module when the batch tables pushed the parent past the
// 1000-line code gate. Nothing here decides anything by itself: every predicate
// is a local syntax aid, and the host stays the authority for semantics.
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
   * `HATCHEDIT <option>` (AutoCAD 2026) is the typed v1 hatch edit. The
   * implemented forms are `ANGLE <degrees>` (one finite signed decimal),
   * `SCALE <factor>` (one finite decimal > 0), `SOLID`, `GRADIENT`,
   * `ADDBOUNDARIES` and `REMOVEBOUNDARIES` (the other selected closed entities
   * become loops, or the stored loops matching them are removed; one undo
   * step). The bare `HATCHEDIT` is admitted by the base name and refused by the
   * host with `hatchedit.needs_option`; every other AutoCAD dialog field
   * (`Pattern`, `Color`, `Properties`, `Draw order`, associativity) is NOT
   * IMPLEMENTED and is refused locally.
   */
  HATCHEDIT: Object.freeze({
    parse: (argument: string) => {
      const parts = argument.trim().toUpperCase().split(/\s+/).filter(part => part.length > 0);
      const [word, ...rest] = parts;
      if (word === 'SOLID' || word === 'GRADIENT' || word === 'ADDBOUNDARIES'
        || word === 'REMOVEBOUNDARIES') {
        return rest.length === 0;
      }
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
  /**
   * `PAGESETUP <layout> <paper> <width> <height> [<num>:<den>] [0|90|180|270]`:
   * one authored page setup for an existing layout. `<layout>` and `<paper>` are
   * single tokens (the host grammar boundary), the dimensions are positive
   * decimals, the optional scale is two positive decimal components and the
   * optional rotation is exactly one of the four quarter turns. `-PAGESETUP` is
   * a distinct extended selector with the same grammar, not an alias.
   */
  PAGESETUP: Object.freeze({
    parse: parsePageSetupArgument,
  }),
  '-PAGESETUP': Object.freeze({
    parse: parsePageSetupArgument,
  }),
  /** `PSETUPIN <source-layout> <target-layout>`: same-drawing copy, two names. */
  PSETUPIN: Object.freeze({
    parse: (argument: string) => tokens(argument).length === 2,
  }),
  '-PSETUPIN': Object.freeze({
    parse: (argument: string) => tokens(argument).length === 2,
  }),
  /** `MVIEW <minx> <miny> <maxx> <maxy>`: one non-degenerate rectangle. */
  MVIEW: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length !== 4 || !parts.every(isSignedDecimal)) return false;
      return Number(parts[0]) < Number(parts[2]) && Number(parts[1]) < Number(parts[3]);
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
  // The 2026-11-07 annotation/group batch typed forms.
  GROUP: Object.freeze({ parse: isGroupArgument }),
  '-GROUP': Object.freeze({ parse: isGroupArgument }),
  UNGROUP: Object.freeze({ parse: isOneName }),
  GROUPEDIT: Object.freeze({ parse: isGroupEditArgument }),
  QLEADER: Object.freeze({ parse: isLeaderArgument }),
  LEADER: Object.freeze({ parse: isLeaderArgument }),
  TEXTFIT: Object.freeze({ parse: (argument: string) => isSignedDecimal(argument.trim()) }),
  HATCHSETORIGIN: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 2 && parts.every(isSignedDecimal);
    },
  }),
  /**
   * `ARRAYPATH <count> [ALIGN]` (2026-11-10): the typed 2D path-array form. The
   * current selection is the input — the first selected entity is the path, the
   * rest are items — so only the count and the optional `ALIGN` word are grammar
   * here. `<count>` is the number of **copies** (the N source objects stay). The
   * count is one positive safe integer no larger than the host's
   * `MAX_ARRAY_ENTITIES` cap (`100_000`); the host re-checks both the count and
   * the total `count × items`.
   */
  ARRAYPATH: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length === 0 || parts.length > 2) return false;
      if (!/^[0-9]+$/.test(parts[0])) return false;
      const count = Number(parts[0]);
      if (!Number.isSafeInteger(count) || count <= 0 || count > 100_000) return false;
      return parts.length === 1 || parts[1].toUpperCase() === 'ALIGN';
    },
  }),
  /**
   * `IMAGEATTACH <file> <x> <y> <width_px> <height_px> <units_per_pixel>`: the
   * file is one token (a path with spaces cannot be typed in v1), the point is a
   * pair of signed decimals, and the pixel size plus the units-per-pixel scale
   * are positive decimals. The host re-checks the reference (safe relative key)
   * and every value; nothing is clamped here.
   */
  IMAGEATTACH: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      return parts.length === 6
        && isSignedDecimal(parts[1]) && isSignedDecimal(parts[2])
        && isPositiveDecimal(parts[3]) && isPositiveDecimal(parts[4])
        && isPositiveDecimal(parts[5]);
    },
  }),
  /**
   * `IMAGECLIP <minx> <miny> <maxx> <maxy>` or `IMAGECLIP OFF`: four signed
   * decimals in the image's pixel space, or the `OFF` word. Ordering
   * (`minx < maxx`, `miny < maxy`) is the host's `imageclip.invalid_rect`
   * refusal, so it is deliberately not rejected locally.
   */
  IMAGECLIP: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      if (parts.length === 1) return parts[0].toUpperCase() === 'OFF';
      return parts.length === 4 && parts.every(isSignedDecimal);
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

/** `GROUP`: bare `CREATE <name>`, `LIST`, `DELETE <name>`; names are one token. */
function isGroupArgument(argument: string): boolean {
  const parts = tokens(argument);
  const word = (parts[0] ?? '').toUpperCase();
  if (word === 'LIST') return parts.length === 1;
  return (word === 'CREATE' || word === 'DELETE') && parts.length === 2;
}

/** `UNGROUP`: one non-empty, whitespace-free group name. */
function isOneName(argument: string): boolean {
  return tokens(argument).length === 1;
}

/** `GROUPEDIT <name> ADD|REMOVE [ids]|RENAME <name>`. */
function isGroupEditArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length < 2) return false;
  const verb = parts[1].toUpperCase();
  if (verb === 'RENAME') return parts.length === 3;
  if (verb !== 'ADD' && verb !== 'REMOVE') return false;
  return parts.slice(2).every(part => /^\d+(,\d+)*$/.test(part));
}

/** `QLEADER`/`LEADER`: `<arrow x,y> <landing x,y> <text...>`. */
function isLeaderArgument(argument: string): boolean {
  const parts = tokens(argument);
  return parts.length >= 3
    && isCoordinatePair(parts[0]) && isCoordinatePair(parts[1]);
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

/**
 * `PAGESETUP`/`-PAGESETUP`: `<layout> <paper> <width> <height> [<num>:<den>]
 * [0|90|180|270]`.
 *
 * Mirrors the host parser's single-token names, positive paper dimensions, two
 * positive scale components and four typed rotations. The optional tail is
 * classified by the presence of `:` (a scale) or by being one of the four
 * rotation words; a second occurrence of either is refused locally.
 */
function parsePageSetupArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length < 4 || parts.length > 6) return false;
  if (!isPositiveDecimal(parts[2]) || !isPositiveDecimal(parts[3])) return false;
  let scale = false;
  let rotation = false;
  for (const part of parts.slice(4)) {
    if (part.includes(':')) {
      if (scale) return false;
      const ratio = part.split(':');
      if (ratio.length !== 2 || !ratio.every(isPositiveDecimal)) return false;
      scale = true;
    } else {
      if (rotation || !['0', '90', '180', '270'].includes(part)) return false;
      rotation = true;
    }
  }
  return true;
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
