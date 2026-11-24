// The typed-argument grammar `gating.ts` exposes as `ARGUMENT_COMMANDS`, split
// verbatim into a sibling module when the batch tables pushed the parent past the
// 1000-line code gate. Nothing here decides anything by itself: every predicate
// is a local syntax aid, and the host stays the authority for semantics.
//
// The clip-family predicates (`CLIP`/`XCLIP`) and the two token helpers they
// share with this table live in `argument-clip-grammar.ts`, so the two clip
// grammars sit side by side and this table stays inside the code-size gate. The
// `HATCHEDIT` option grammar lives in `argument-hatchedit.ts` for the same reason,
// the `COLOR`/`-COLOR` token grammar in `argument-color-grammar.ts` (one predicate
// shared by both spellings), and the MULTILEADER/`SSX`/record grammars in
// `argument-{mleader,selection,record}-grammar.ts`. The `LINETYPE`/`MLINE`/
// `LAYOUT`/`TABLEEXPORT` shapes — plus the quote-aware tokenizer they share with
// `RENAME` — live in `argument-extras-grammar.ts`.
import { isHatchEditArgument } from './argument-hatchedit.ts';
import { isColorArgument } from './argument-color-grammar.ts';
import { EXPORT_ARGUMENT_SPEC } from './argument-export-grammar.ts';
import {
  isApertureArgument,
  isPtypeArgument,
  isWipeoutFrameArgument,
} from './argument-session-grammar.ts';
import {
  isClipArgument, isPositiveDecimal, isSignedDecimal, isXclipArgument, tokens,
} from './argument-clip-grammar.ts';
import {
  isMLeaderAlignArgument, isMLeaderEditArgument, isMlEditArgument, isToleranceArgument,
} from './argument-mleader-grammar.ts';
import {
  isDashParametersArgument, isLayoutArgument, isLinetypeArgument, isMlineArgument,
  isTableExportArgument, splitQuotedTokens,
} from './argument-extras-grammar.ts';
import { isAddSelectedArgument, isAliasEditArgument, isMvSetupArgument } from './argument-record-grammar.ts';
import { isSsxArgument } from './argument-selection-grammar.ts';

/** The parser for the one typed-argument form a selector accepts. */
export interface ArgumentSpec {
  readonly parse: (argument: string) => boolean;
}

/**
 * Whether `text` is one `ZOOM` argument from the host's grammar: a positive
 * decimal factor, `ALL`/`EXTENTS`, `WINDOW`, `PREVIOUS`, `OBJECT`, `nX`, or
 * `nXP` / `a/bXP` / `a:bXP`.
 *
 * This is the local **syntax** gate only. The host resolves the selection for
 * `OBJECT`, validates the factor and the 2D view, and routes `nXP` through the
 * Work-only `VPSCALE` write (a model-space `nXP` is refused there). `DYNAMIC`
 * is deliberately absent: it is a named unimplemented gap.
 */
function isZoomArgument(text: string): boolean {
  const key = text.trim();
  const upper = key.toUpperCase();
  if (['ALL', 'EXTENTS', 'WINDOW', 'PREVIOUS', 'OBJECT'].includes(upper)) return true;
  if (isPositiveDecimal(key)) return true;
  if (upper.endsWith('XP')) {
    const body = key.slice(0, -2);
    const ratio = body.split(/[/:]/);
    return ratio.length === 1
      ? isPositiveDecimal(ratio[0])
      : ratio.length === 2 && isPositiveDecimal(ratio[0]) && isPositiveDecimal(ratio[1]);
  }
  if (upper.endsWith('X')) return isPositiveDecimal(key.slice(0, -1));
  return false;
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
 * Whether `text` is a finite decimal with an optional sign that is **greater
 * than zero** (`2`, `0.5`, `+3`; never `0`, `-1`).
 *
 * `MOCORO SCALE` needs this shape: the host's grammar is the same plain decimal
 * plus the strict positivity rule, so a zero/negative factor is refused locally
 * instead of reaching the router.
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
     * `ZOOM WINDOW`, `ZOOM PREVIOUS`, `ZOOM OBJECT`, `ZOOM nX` and `ZOOM nXP`
     * are **argument forms of the `ZOOM` selector**, not commands of their own:
     * the host adds no `CommandId` and advertises no extra name, so the words
     * are grammar here exactly like `ALL`/`EXTENTS` are, and the line still
     * travels as one `cad_command` (`"ZOOM OBJECT"`). They are alternatives in
     * the same grammar as the decimal factor, so no word can ever be read as one
     * (`isPositiveDecimal` already refuses letters) and no new selector row is
     * needed.
     *
     * This table is the *only* place the words are listed for admission: the
     * command line admits them through the ordinary base-name path
     * (`splitCommandText` reads `ZOOM` + `WINDOW`), and the ribbon's
     * `shell.canCommand('ZOOM OBJECT')` is that same call — so the button is
     * enabled iff `ZOOM` itself is admitted, with no second name list and no
     * `HOST_NAMES` entry the host does not publish. Semantics stay the host's:
     * a 3D view refuses `WINDOW`/`OBJECT` by name, `PREVIOUS` with no recorded
     * step is a diagnostic, `OBJECT` needs a selection, and `nXP` requires an
     * active paper layout (it is a Work-only `VPSCALE` write). `DYNAMIC` is not
     * admitted anywhere: it is a named unimplemented gap.
     */
    parse: isZoomArgument,
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
   * `HATCHEDIT <option>` (AutoCAD 2026): the typed hatch edit.
   *
   * The predicate is `isHatchEditArgument` (in `argument-hatchedit.ts`), which
   * mirrors the host parser token for token: `STYLE IGNORE|OUTER|NORMAL`,
   * `ANGLE <degrees>`, `SCALE <factor>`, `PATTERN <name>`,
   * `PATTERNFILE <path.pat> [<name>]`, `PATTERNTYPE USER|PREDEFINED|CUSTOM`,
   * `SOLID`, `GRADIENT`, `COLOR <aci>|BYLAYER`,
   * `TRANSPARENCY <0..=90>|BYLAYER`, `LAYER <name>`,
   * `ORIGIN <x> <y>`, `DRAWORDER BEHIND|FRONT|BOUNDARY`, `RECREATEBOUNDARY`,
   * `SEPARATE`, `ADDBOUNDARIES`, `REMOVEBOUNDARIES`, plus the named-refusal forms
   * `ASSOCIATE`/`DISASSOCIATE`/`ANNOTATIVE` (forwarded so the **host** names the
   * missing model). The bare `HATCHEDIT` is admitted by the base name and refused
   * by the host with `hatchedit.needs_option`.
   *
   * Nothing semantic is decided here: the pattern-name table, the pattern-fill
   * requirement, layer existence and reading a `PATTERNFILE` path are all the
   * host's.
   */
  HATCHEDIT: Object.freeze({ parse: isHatchEditArgument }),
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
    // The 15 alignment spellings (the original six single-axis words plus
    // AutoCAD 2026's nine combined TEXTALIGNMODE words), `DISTRIBUTE` with the
    // default Y axis or an explicit X|Y, and the three spacing words the host
    // accepts and then refuses by code. Admission is local; the host is the
    // write authority and every refusal comes from the session.
    parse: (argument: string) => {
      const parts = argument.trim().toUpperCase().split(/\s+/).filter(part => part.length > 0);
      if (parts.length === 1) {
        const keyword = parts[0];
        return [
          'LEFT', 'RIGHT', 'CENTER', 'TOP', 'MIDDLE', 'BOTTOM',
          'TOPLEFT', 'TOPCENTER', 'TOPRIGHT',
          'MIDDLELEFT', 'MIDDLECENTER', 'MIDDLERIGHT',
          'BOTTOMLEFT', 'BOTTOMCENTER', 'BOTTOMRIGHT',
          'DISTRIBUTE', 'SETSPACING', 'CURRENTVERTICAL', 'CURRENTHORIZONTAL',
        ].includes(keyword);
      }
      return parts.length === 2
        && parts[0] === 'DISTRIBUTE'
        && ['X', 'Y'].includes(parts[1]);
    },
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
  COLOR: Object.freeze({
    // The session current-colour token: one ACI or one symbolic word (both
    // spellings share `isColorArgument`). The closed `0..=256` domain is the
    // host's, so `COLOR 257` travels and is named there (`color.invalid_value`)
    // rather than becoming a local "unsupported".
    parse: isColorArgument,
  }),
  '-COLOR': Object.freeze({
    // AutoCAD's prompt spelling of the same token, same grammar: `-COLOR 5`.
    parse: isColorArgument,
  }),
  // The remaining session tokens and session-only vocabulary controls: a bare
  // name reports or keeps its handler default, and one value is admitted here
  // with its **domain** left to the host (`wipeoutframe.out_of_range`,
  // `aperture.*`, `ptype.invalid_argument`, the vocabulary's own refusals).
  WIPEOUTFRAME: Object.freeze({ parse: isWipeoutFrameArgument }),
  APERTURE: Object.freeze({ parse: isApertureArgument }),
  PTYPE: Object.freeze({ parse: isPtypeArgument }),
  REVCLOUDPROPERTIES: Object.freeze({ parse: isPositiveDecimal }),
  UNDEFINE: Object.freeze({ parse: isOneName }),
  REDEFINE: Object.freeze({ parse: isOneName }),
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
  /**
   * `-LAYER <option> …` (AutoCAD's command-line layer editor): the typed write
   * forms `NEW|N`, `MAKE|M`, `SET|S`, `RENAME|R old new`, `DELETE|D`, `ON`,
   * `OFF`, `FREEZE|F`, `THAW|T`, `LOCK|LO`, `UNLOCK|U`, `COLOR|C <value>`,
   * `LTYPE|L <name>`, `LWEIGHT|LW <value>` and `PLOT|P Y|N`, each followed by
   * comma-separated layer names. The bare selector and `?` stay the read-only
   * table report.
   *
   * Only the presence of a non-empty argument is checked here: the grammar, the
   * name resolution and every refusal belong to the Rust parser and the command
   * layer (the host re-validates the whole line), so the chrome must not reject
   * a line it cannot judge — e.g. a layer name containing spaces.
   */
  '-LAYER': Object.freeze({
    parse: (argument: string) => argument.trim().length > 0,
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
  /**
   * `SYSVARMONITOR ADD|REMOVE [<name>]` / `LIST [<name>]`: the session watch set.
   * Only the line shape is checked here; the modelled name set and the stable
   * `sysvarmonitor.*` refusals belong to the host.
   */
  SYSVARMONITOR: Object.freeze({
    parse: (argument: string) => {
      const parts = tokens(argument);
      const word = (parts[0] ?? '').toUpperCase();
      if (word === 'LIST') return parts.length <= 2;
      if (word !== 'ADD' && word !== 'REMOVE') return false;
      return parts.length === 2;
    },
  }),
  /** `SPACETRANS <distance>`: one positive plain decimal model distance. */
  SPACETRANS: Object.freeze({
    parse: isPositiveDecimal,
  }),
  // Navigation-chrome settings. The five names are host setting selectors, not
  // drawing commands: `NAVBAR`/`NAVVCUBE` take On/Off, the display sysvar takes
  // the AutoCAD 0..3 bit code, and the two registry gates take On/Off. The host
  // remains the semantic authority; this only admits the line shape.
  NAVBAR: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF', '1', '0'].includes(argument.trim().toUpperCase()),
  }),
  NAVVCUBE: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF', '1', '0', 'PROPERTIES']
      .includes(argument.trim().toUpperCase()),
  }),
  NAVVCUBEDISPLAY: Object.freeze({
    parse: (argument: string) => /^[0-3]$/.test(argument.trim()),
  }),
  DISPLAYVIEWCUBEIN2D: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF', '1', '0'].includes(argument.trim().toUpperCase()),
  }),
  DISPLAYVIEWCUBEIN3D: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF', '1', '0'].includes(argument.trim().toUpperCase()),
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
  /**
   * `MLINE <spacing> <offset> <x1,y1> <x2,y2> [<x3,y3> …]`: a positive spacing,
   * a signed offset and two or more baseline vertices (the grammar lives in
   * `argument-extras-grammar.ts`). The `2..=1024` vertex bound is the command
   * layer's `mline.point_count` refusal, so it is deliberately not imposed here.
   */
  MLINE: Object.freeze({
    parse: isMlineArgument,
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
  /**
   * `LINETYPE LIST|SET <name>|DELETE <name>|CREATE <name> <dash>,<gap>,…
   * [DESCRIPTION <text>]`; `-LINETYPE` is the command-line spelling of the same
   * grammar (one predicate for both rows, exactly like `-PAGESETUP`). The
   * `CREATE` tail's `DESCRIPTION` text is content, so admission preserves the
   * typed case through `CASE_PRESERVING_ARGUMENTS` in `commands.ts`; whether the
   * name exists, whether the pattern is a valid dash/gap sequence and the table's
   * limits stay `linetype.*` refusals.
   */
  LINETYPE: Object.freeze({
    parse: isLinetypeArgument,
  }),
  '-LINETYPE': Object.freeze({
    parse: isLinetypeArgument,
  }),

  /**
   * `LAYOUT LIST|NEW|RENAME|DELETE|SET|COPY <source> <new-name>|TEMPLATE …`.
   * `TEMPLATE` stays admitted so the host can answer its own
   * `layout.template_not_implemented` instead of the chrome's generic refusal.
   */
  LAYOUT: Object.freeze({
    parse: isLayoutArgument,
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
  /**
   * `XCLIP RECT <minx> <miny> <maxx> <maxy>`, `XCLIP POLY <x1> <y1> …`,
   * `XCLIP ON|OFF|DELETE|INVERT` or `XCLIP ?`: the **display-only**
   * block-reference clip on exactly one selected INSERT. Every coordinate is
   * **block-local**; the host owns the rectangle ordering
   * (`xclip.degenerate_rect`), polygon validity, the axis-aligned world mapping
   * (`xclip.rotation_not_renderable` / `.non_xy_plane`) and the declared render
   * limits (`INVERT` and polygon boundaries are refused/reported there), so this
   * predicate checks only the token shape. A bare `XCLIP` stays name-only.
   */
  XCLIP: Object.freeze({
    parse: isXclipArgument,
  }),
  /**
   * `CLIP RECT|POLY|ON|OFF|DELETE`: the generic boundary form over the same two
   * clip models `IMAGECLIP`/`XCLIP` write, on exactly one selected image or
   * block reference. The target kind, the target's clip space, the "images have
   * no ON/OFF" rule and every geometry refusal stay the host's (`clip.*`,
   * `imageclip.*`, `xclip.*`). A bare `CLIP` stays name-only; `CLIPIT` takes no
   * argument at all.
   */
  CLIP: Object.freeze({
    parse: isClipArgument,
  }),
  /**
   * `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` or
   * `IMAGEADJUST RESET`: the display transfer of the one selected image. The
   * three percentages are plain integers (the host owns the `0..=100` range and
   * refuses an overshoot by name), and the optional word is `MONO`/`COLOR`.
   * `RESET` restores 50/50/0/off. `-IMAGEADJUST` takes the same grammar.
   */
  IMAGEADJUST: Object.freeze({
    parse: isImageAdjustArgument,
  }),
  '-IMAGEADJUST': Object.freeze({
    parse: isImageAdjustArgument,
  }),
  /**
   * `TABLE <rows> <cols> <col_width> <row_height> <x> <y>` and its `-TABLE`
   * spelling: an empty table grid on the world XY plane. The counts are positive
   * integers, the two cell sizes are positive decimals and the origin is a pair
   * of signed decimals. The host owns the 1000 × 1000 / 100 000-cell caps and
   * every semantic refusal (`table.invalid_dimensions` / `.create_too_large` /
   * `.invalid_origin` / `.invalid_size`), so nothing is clamped here.
   */
  TABLE: Object.freeze({
    parse: isTableCreateArgument,
  }),
  '-TABLE': Object.freeze({
    parse: isTableCreateArgument,
  }),
  /**
   * `TABLEDIT <row> <col> [text...]`: a 1-based cell address on the single
   * selected table. The text is the remainder of the line and may contain
   * spaces (it is preserved by the host); an empty remainder clears the cell.
   * The host owns the range and selection refusals (`tabledit.out_of_range` /
   * `.no_selection` / `.multiple` / `.not_table`).
   */
  TABLEDIT: Object.freeze({
    parse: isTableEditArgument,
  }),
  /**
   * `TABLEEXPORT <path.csv>`: the destination is typed (AutoCAD opens a dialog;
   * this build has none) and the host writes the CSV through its file channel,
   * like `SAVE`/`PLOT`. The whole remainder is the path, so its typed case is
   * preserved (`CASE_PRESERVING_ARGUMENTS`); the `.csv` requirement, the
   * one-table selection rule, a runtime with no filesystem and a failed write are
   * all named host refusals (`tableexport.*`, `unsupported-runtime:
   * table-export`) that this predicate must not pre-empt.
   */
  TABLEEXPORT: Object.freeze({
    parse: isTableExportArgument,
  }),

  // The 2026-11-15 audit batch: nine selectors whose host parser already took the
  // typed argument while this table had no row (shapes in `argument-*-grammar.ts`;
  // every documented model boundary is unchanged and stays the host's refusal).
  ADDSELECTED: Object.freeze({ parse: isAddSelectedArgument }),
  ALIASEDIT: Object.freeze({ parse: isAliasEditArgument }),
  MLEDIT: Object.freeze({ parse: isMlEditArgument }),
  '-MLEDIT': Object.freeze({ parse: isMlEditArgument }),
  MLEADERALIGN: Object.freeze({ parse: isMLeaderAlignArgument }),
  MLEADEREDIT: Object.freeze({ parse: isMLeaderEditArgument }),
  MVSETUP: Object.freeze({ parse: isMvSetupArgument }),
  TOLERANCE: Object.freeze({ parse: isToleranceArgument }),
  SSX: Object.freeze({ parse: isSsxArgument }),
  /**
   * `IBL [ON|OFF|1|0]`: the renderer's image-based lighting switch for PBR
   * meshes. A bare `IBL` toggles and stays on the name-only funnel (the host
   * reads the current state); the explicit forms are admitted here,
   * case-insensitively, and everything else is the shared
   * `command.unsupported` refusal before IPC. Appended last so no shipped row
   * moves.
   */
  IBL: Object.freeze({
    parse: (argument: string) => ['ON', 'OFF', '1', '0'].includes(argument.trim().toUpperCase()),
  }),
  // `EXPORT <format>`: the typed format token (grammar in `argument-export-grammar.ts`).
  EXPORT: EXPORT_ARGUMENT_SPEC,
  '-PARAMETERS': Object.freeze({ parse: isDashParametersArgument }),
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

/**
 * `TABLE`/`-TABLE`: positive integer counts, positive cell sizes and a signed
 * origin pair. This is the six-token shape the host grammar admits; the caps
 * and every semantic refusal belong to the command layer.
 */
function isTableCreateArgument(argument: string): boolean {
  const parts = tokens(argument);
  return parts.length === 6
    && /^[0-9]+$/.test(parts[0]) && /^[0-9]+$/.test(parts[1])
    && Number(parts[0]) > 0 && Number(parts[1]) > 0
    && isPositiveDecimal(parts[2]) && isPositiveDecimal(parts[3])
    && isSignedDecimal(parts[4]) && isSignedDecimal(parts[5]);
}

/**
 * `TABLEDIT <row> <col> [text...]`: a 1-based cell address plus an optional
 * text remainder. The remainder is the whole rest of the line (spaces
 * included) and is never re-tokenised here — the host preserves it verbatim.
 */
function isTableEditArgument(argument: string): boolean {
  const match = /^\s*([0-9]+)[ \t]+([0-9]+)(?:[ \t]+[\s\S]*)?$/.exec(argument);
  if (match === null) return false;
  return Number(match[1]) > 0 && Number(match[2]) > 0;
}

/**
 * `IMAGEADJUST <brightness> <contrast> <fade> [MONO|COLOR]` or
 * `IMAGEADJUST RESET` (and the `-IMAGEADJUST` spelling).
 *
 * The three percentages are plain non-negative integers in `u8` shape
 * (`0..=255`); the host owns the real `0..=100` range and refuses an overshoot
 * with `imageadjust.invalid_value`, so nothing is clamped or range-checked here.
 * The optional fourth token is the case-insensitive `MONO`/`COLOR` word, and
 * omitting it keeps the image's current monochrome state.
 */
function isImageAdjustArgument(argument: string): boolean {
  const parts = tokens(argument);
  if (parts.length === 1) return parts[0].toUpperCase() === 'RESET';
  if (parts.length !== 3 && parts.length !== 4) return false;
  const percentage = (text: string): boolean =>
    /^[0-9]+$/.test(text) && Number(text) <= 255;
  if (!percentage(parts[0]) || !percentage(parts[1]) || !percentage(parts[2])) return false;
  if (parts.length === 3) return true;
  const word = parts[3].toUpperCase();
  return word === 'MONO' || word === 'COLOR';
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

