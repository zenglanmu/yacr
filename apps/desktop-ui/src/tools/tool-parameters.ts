// The active tool's typed parameters (ADR 0016 decision A): the control spec
// table mirrored from the frozen host table, the status-bar views, the bounded
// pending delta, the typed-entry draft model and the one IPC that writes a value.
//
// Split out of `cad.ts` (which is at the code-size ceiling); `cad.ts` re-exports
// this module, so the chrome's import surface is exactly what it was.
import type { CadStatus, CadTool, Invoke } from '../cad/cad-status.ts';

/**
 * The one general typed tool-parameter IPC (ADR 0016 decision A). It replaces
 * the removed narrow, polygon-only side-count IPC: every parameter now travels
 * this channel, carrying its stable machine key plus exactly one of
 * `number`/`flag`.
 *
 * `{accepted:true}` means **queued**, exactly like `cad_command` — never
 * completed. A malformed request (an unknown key, or neither/both value fields)
 * rejects with a string; the per-parameter range/kind and Viewer-mode refusals
 * surface later through `cad_status.error`.
 */
export const TOOL_PARAMETER_COMMAND = 'cad_set_tool_parameter';

// Cap on the pending delta magnitude, shared by every numeric parameter (it
// replaces the POLYGON-only cap). 10 keeps a stuck queue from accumulating
// unbounded intent — at most ten un-echoed steps, i.e. a ~2.5 s burst at the
// 250 ms poll — while staying well above a normal click cadence, so the host
// virtually always catches up before the cap is ever felt.
export const TOOL_PARAMETER_PENDING_CAP = 10;

/** The host's own per-parameter value predicate (never a clamp). */
type ParameterPredicate = (value: number) => boolean;

// The host's own per-parameter value predicates (`cad_app::ToolParameter::validate`,
// ADR 0016 decision A), mirrored here so the UI never offers a value the host
// will refuse. The host refuses explicitly and never clamps, so an unoffered
// value is simply not shown as a step (and never silently rewritten).
const integerInRange = (min: number, max: number): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && value >= min && value <= max;
const integerAtLeast = (min: number): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && value >= min;
const finiteAtLeast = (min: number): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value >= min;
const finiteAny = (): ParameterPredicate => (value: number): boolean => Number.isFinite(value);
const finiteAbove = (low: number): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value > low;
// A closed set of integer codes (HATCH pattern type / style). The host refuses a
// code outside the set rather than clamping, so only the listed codes are offered.
const integerIn = (...values: number[]): ParameterPredicate =>
  (value: number): boolean => Number.isSafeInteger(value) && values.includes(value);

/** A closed-set parameter's exact accepted integer codes. */
interface ClosedSet {
  readonly choices: readonly number[];
  readonly offered: ParameterPredicate;
}

const closedSet = (...values: number[]): ClosedSet => Object.freeze({
  choices: Object.freeze([...values]),
  offered: integerIn(...values),
});
// Finite, non-zero and within a single turn (0 < |angle| <= 360): 360/-360 (the
// full-circle default) ARE offered targets, while 0 and anything beyond a turn
// (±720, …) are refused — a fill angle is always an explicitly-bounded sweep.
const partialTurn = (): ParameterPredicate =>
  (value: number): boolean => Number.isFinite(value) && value !== 0 && Math.abs(value) <= 360;

/** Fields every tool-parameter control spec shares. */
interface ToolParameterSpecBase {
  /** The field of `cad_status.tool.params` that projects the live value. */
  readonly statusKey: string;
  /** Catalog key of the control's label (present in both catalogs). */
  readonly labelKey: string;
  /** Catalog key of the control's hint (present in both catalogs). */
  readonly hintKey: string;
  /** An optional status-bar icon, present only where one exists. */
  readonly icon?: string;
  /** Whether a projected value may be rendered; defaults to `offered`. */
  readonly projectable?: ParameterPredicate;
  /**
   * The sibling parameter code this row is **scoped to**, or absent when the row
   * always applies.
   *
   * A scoped row stays **visible** — so its choices/values stay discoverable before
   * the sibling is switched — but renders inert, with its `hintKey` (plus
   * [`TOOL_PARAMETER_INACTIVE_KEY`]) as the reason. It is **scope, not capability**:
   * the host would still accept the write, so an inert row never disables dispatch
   * on that ground. An absent or unprojected sibling code never claims the row
   * applies.
   *
   * HATCH: the predefined pattern only means something while the pattern type is
   * `2`, its spacing/angle/`double` only while the fill is one of the pattern kinds
   * (`1` user-defined, `2` predefined), and the gradient rows only while the type
   * is `3` — under any other type the host ignores them.
   */
  readonly activeWhen?: { readonly statusKey: string; readonly values: readonly number[] };
}

/** A `'number'` parameter: a bounded stepper. */
export interface NumberToolParameterSpec extends ToolParameterSpecBase {
  readonly kind: 'number';
  /** The increment a numeric stepper offers (integers step by 1; angles by 15°). */
  readonly step: number;
  /** The host's own predicate: whether it would **accept** a value as a target. */
  readonly offered: ParameterPredicate;
  /** For a closed-set parameter, the exact integer codes the host accepts. */
  readonly choices?: readonly number[];
  /**
   * The catalog key of each code's user-visible **name**, keyed by the code.
   *
   * Only a closed set whose codes a user cannot be expected to decode carries
   * this: `hatchPattern` is `ANSI31`/`ANSI32`/`ANSI37` in the host's own
   * transcribed table, so the strip can name the pattern instead of showing the
   * index. A row without it stays a plain bounded stepper, and its codes are
   * spoken by its `hintKey` alone.
   *
   * Every code in `choices` must carry a name and no other code may appear:
   * [`toolParameterViews`] keeps the stepper when the two disagree, so an
   * incomplete table can neither invent a name nor hide a code the host accepts.
   * The names are copy, so they live in the catalogs — never here.
   */
  readonly choiceLabelKeys?: Readonly<Record<number, string>>;
  /**
   * Whether the value is a **colour** the strip shows as a swatch, not a number.
   *
   * HATCH's two gradient stops are the only such rows: the wire carries a bare
   * integer ACI index (`cad_set_tool_parameter {parameter, number}`), so the row
   * paints that index from the shared ACI palette and opens the shared in-DOM
   * colour chooser. Nothing free-form is offered — a `#RRGGBB` would have nowhere
   * to go — and the chooser's symbolic entries are empty, so ACI `0` (ByBlock, the
   * placeholder with no colour) can never be picked.
   *
   * A swatch row is still a numeric row: the same `offered` predicate guards the
   * write and the same one IPC carries it.
   */
  readonly swatch?: boolean;
  /**
   * Whether the value may also be **typed** into the strip, not only stepped.
   *
   * Only a real length needs it: a text height of `1.75` is a value no fixed
   * increment reaches, while a side count or a HATCH code is naturally stepped.
   * A typed value travels the same one IPC through the same `offered` guard, so
   * typing can never reach a value a step could not, and an unoffered one is
   * refused before dispatch (see [`commitToolParameterDraft`]).
   */
  readonly typed?: boolean;
}

/** A `'flag'` parameter: a toggle. */
export interface FlagToolParameterSpec extends ToolParameterSpecBase {
  readonly kind: 'flag';
}

/** The per-parameter control spec, mirrored from the frozen host table. */
export type ToolParameterSpec = NumberToolParameterSpec | FlagToolParameterSpec;

/** Every parameter spec, keyed by its stable machine key. */
export type ToolParameterSpecMap = Readonly<Record<string, ToolParameterSpec>>;

/**
 * The per-parameter control spec, mirrored from the frozen host table
 * (`cad_app::draw_tool::parameter`, ADR 0016 decision A).
 *
 * - `kind`      — `'number'` (bounded stepper) or `'flag'` (toggle);
 * - `statusKey` — the field of `cad_status.tool.params` that projects the live
 *   value (POLYGON's legacy `sides`; later phases project each parameter under
 *   its own machine key);
 * - `step`      — the increment a numeric stepper offers (integer parameters step
 *   by 1; the fill angle steps by 15°);
 * - `offered`   — the host's own predicate: whether the host would **accept** a
 *   value as a target (the stepper never offers a value this rejects);
 * - `choices`   — for a closed-set numeric parameter, the exact integer codes the
 *   host accepts (and therefore the only ones the stepper can reach);
 * - `choiceLabelKeys` — the catalog key of each code's name, for a closed set a
 *   user reads by name (the predefined HATCH patterns). Such a row renders a
 *   named chooser instead of a stepper; the value on the wire is unchanged, so
 *   the choice's name and the integer index are the same choice;
 * - `activeWhen` — the sibling code a row only applies under (the pattern name
 *   under the predefined pattern type, the gradient rows under the gradient type,
 *   the pattern-only rows under the two pattern types). The row stays visible but
 *   inert while the sibling says otherwise; its hint (and
 *   [`TOOL_PARAMETER_INACTIVE_KEY`]) says why, rather than the control
 *   disappearing;
 * - `swatch`    — the row's value is an integer ACI index shown as a colour
 *   swatch (HATCH's two gradient stops), not a number to step. What travels on the
 *   wire is unchanged: the same integer through the same one IPC;
 * - `projectable` — whether a value from `cad_status` may be **rendered**. It
 *   defaults to `offered` (an out-of-range projection is a contract violation and
 *   renders nothing), so no parameter currently overrides it;
 * - `labelKey` / `hintKey` — catalog keys (present in both catalogs);
 * - `icon`      — an optional status-bar icon, present only where one exists.
 */
export const TOOL_PARAMETER_SPECS: ToolParameterSpecMap = Object.freeze({
  polygonSides: {
    kind: 'number', statusKey: 'sides', step: 1, icon: 'polygon',
    offered: integerInRange(3, 1024),
    labelKey: 'status.tool_parameter.polygon_sides',
    hintKey: 'status.tool_parameter.polygon_sides.hint',
  },
  filletRadius: {
    kind: 'number', statusKey: 'filletRadius', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.fillet_radius',
    hintKey: 'status.tool_parameter.fillet_radius.hint',
  },
  chamferDistanceA: {
    kind: 'number', statusKey: 'chamferDistanceA', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.chamfer_distance_a',
    hintKey: 'status.tool_parameter.chamfer_distance_a.hint',
  },
  chamferDistanceB: {
    kind: 'number', statusKey: 'chamferDistanceB', step: 1,
    offered: finiteAtLeast(0),
    labelKey: 'status.tool_parameter.chamfer_distance_b',
    hintKey: 'status.tool_parameter.chamfer_distance_b.hint',
  },
  alignScale: {
    kind: 'flag', statusKey: 'alignScale',
    labelKey: 'status.tool_parameter.align_scale',
    hintKey: 'status.tool_parameter.align_scale.hint',
  },
  arrayRows: {
    kind: 'number', statusKey: 'arrayRows', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_rows',
    hintKey: 'status.tool_parameter.array_rows.hint',
  },
  arrayColumns: {
    kind: 'number', statusKey: 'arrayColumns', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_columns',
    hintKey: 'status.tool_parameter.array_columns.hint',
  },
  arrayRowSpacing: {
    kind: 'number', statusKey: 'arrayRowSpacing', step: 1,
    offered: finiteAny(),
    labelKey: 'status.tool_parameter.array_row_spacing',
    hintKey: 'status.tool_parameter.array_row_spacing.hint',
  },
  arrayColumnSpacing: {
    kind: 'number', statusKey: 'arrayColumnSpacing', step: 1,
    offered: finiteAny(),
    labelKey: 'status.tool_parameter.array_column_spacing',
    hintKey: 'status.tool_parameter.array_column_spacing.hint',
  },
  arrayItems: {
    kind: 'number', statusKey: 'arrayItems', step: 1,
    offered: integerAtLeast(1),
    labelKey: 'status.tool_parameter.array_items',
    hintKey: 'status.tool_parameter.array_items.hint',
  },
  arrayFillAngle: {
    kind: 'number', statusKey: 'arrayFillAngle', step: 15,
    offered: partialTurn(),
    labelKey: 'status.tool_parameter.array_fill_angle',
    hintKey: 'status.tool_parameter.array_fill_angle.hint',
  },
  arrayRotateItems: {
    kind: 'flag', statusKey: 'arrayRotateItems',
    labelKey: 'status.tool_parameter.array_rotate_items',
    hintKey: 'status.tool_parameter.array_rotate_items.hint',
  },
  // HATCH (ADR 0018 decision D, P6b; gradient channel 2026-10-27). The rows
  // mirror `cad_app::draw_tool::parameter`: the closed sets use `choices` so a
  // control can only reach the implemented codes, spacing must be finite and
  // strictly positive (it doubles as the pattern scale / `HPSCALE`), the angle is
  // any finite number of degrees (no implicit wrap — the host accepts it as
  // typed) and `double` is a flag.
  //
  // The four fill kinds are the closed set `0` solid, `1` user-defined, `2`
  // predefined, `3` gradient. Pattern type `2` selects one of the **three**
  // predefined patterns the host transcribed (`ANSI31`, `ANSI32`, `ANSI37`) by its
  // 1-based index; type `3` is a two-colour gradient whose own four rows live
  // below. Custom pattern names, `.pat` files and associativity still have no
  // parameter here because they are not implemented.
  hatchPatternType: {
    kind: 'number', statusKey: 'hatchPatternType', step: 1,
    ...closedSet(0, 1, 2, 3),
    labelKey: 'status.tool_parameter.hatch_pattern_type',
    hintKey: 'status.tool_parameter.hatch_pattern_type.hint',
  },
  hatchPattern: {
    // The 1-based index of the implemented predefined pattern table. The upper
    // bound is the host table length (ANSI31, ANSI32, ANSI37), so an offered
    // value can never name a pattern the host does not implement.
    kind: 'number', statusKey: 'hatchPattern', step: 1,
    ...closedSet(1, 2, 3),
    // The three implemented patterns are read **by name**: the wire keeps the
    // 1-based index (1 ANSI31, 2 ANSI32, 3 ANSI37), and the strip offers the names
    // as a segmented chip, so nothing has to be decoded from a number.
    choiceLabelKeys: Object.freeze({
      1: 'status.tool_parameter.hatch_pattern.ansi31',
      2: 'status.tool_parameter.hatch_pattern.ansi32',
      3: 'status.tool_parameter.hatch_pattern.ansi37',
    }),
    // A pattern name only applies to the predefined type (2): solid, user-defined
    // and gradient fills ignore the index, so under those codes the chooser stays
    // visible but inert and its hint says why.
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([2]) }),
    labelKey: 'status.tool_parameter.hatch_pattern',
    hintKey: 'status.tool_parameter.hatch_pattern.hint',
  },
  // --- the gradient fill (pattern type 3) -----------------------------------
  // The gradient kind is a **named** chooser, like the pattern names: the two
  // codes are two ramps a user reads by name, and a raw 0/1 stepper would be a
  // number nobody can decode. The domain's third spelling (`CYLINDER`) is
  // deliberately absent — it bakes as spherical, so offering it would be a second
  // name for one behaviour.
  hatchGradientKind: {
    kind: 'number', statusKey: 'hatchGradientKind', step: 1,
    ...closedSet(0, 1),
    choiceLabelKeys: Object.freeze({
      0: 'status.tool_parameter.hatch_gradient_kind.linear',
      1: 'status.tool_parameter.hatch_gradient_kind.spherical',
    }),
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([3]) }),
    labelKey: 'status.tool_parameter.hatch_gradient_kind',
    hintKey: 'status.tool_parameter.hatch_gradient_kind.hint',
  },
  hatchGradientAngle: {
    // Degrees, like the pattern angle (the host converts to radians exactly once
    // when it builds the entity), so the two angle rows step by the same 15° and
    // any finite value is accepted — no implicit wrap.
    kind: 'number', statusKey: 'hatchGradientAngle', step: 15,
    offered: finiteAny(),
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([3]) }),
    labelKey: 'status.tool_parameter.hatch_gradient_angle',
    hintKey: 'status.tool_parameter.hatch_gradient_angle.hint',
  },
  hatchGradientColorA: {
    // A bare integer ACI index on the wire (the channel has no colour type), shown
    // as a swatch. `1..=255` — deliberately narrower than the channel's `0..=255`,
    // because index `0` is the ByBlock placeholder with no sRGB triple and the
    // command refuses it by name; the row must never offer a value the drawing
    // would refuse. `step` is unused (a swatch has no `-`/`+`), kept at 1 so the
    // row stays an ordinary numeric row.
    kind: 'number', statusKey: 'hatchGradientColorA', step: 1,
    offered: integerInRange(1, 255),
    swatch: true,
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([3]) }),
    labelKey: 'status.tool_parameter.hatch_gradient_color_a',
    hintKey: 'status.tool_parameter.hatch_gradient_color_a.hint',
  },
  hatchGradientColorB: {
    // The ramp's end stop; the same ACI rule as the start stop.
    kind: 'number', statusKey: 'hatchGradientColorB', step: 1,
    offered: integerInRange(1, 255),
    swatch: true,
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([3]) }),
    labelKey: 'status.tool_parameter.hatch_gradient_color_b',
    hintKey: 'status.tool_parameter.hatch_gradient_color_b.hint',
  },
  // --- pattern-only rows ----------------------------------------------------
  // Spacing, angle and double exist for the two pattern kinds only: the command
  // never consults them for a solid or a gradient fill, so under those codes the
  // rows stay visible but inert (scope, never capability — a write is still
  // accepted, the host simply ignores it).
  hatchSpacing: {
    // A real length: the 1-unit step every other length uses would make every
    // spacing below the default 1 unreachable, so this one steps by 0.5. The
    // host accepts any finite spacing > 0; it is also the predefined pattern
    // scale (`HPSCALE`). The stepper cannot enter arbitrary decimals (there is
    // no free-entry field).
    kind: 'number', statusKey: 'hatchSpacing', step: 0.5,
    offered: finiteAbove(0),
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([1, 2]) }),
    labelKey: 'status.tool_parameter.hatch_spacing',
    hintKey: 'status.tool_parameter.hatch_spacing.hint',
  },
  hatchAngle: {
    kind: 'number', statusKey: 'hatchAngle', step: 15,
    offered: finiteAny(),
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([1, 2]) }),
    labelKey: 'status.tool_parameter.hatch_angle',
    hintKey: 'status.tool_parameter.hatch_angle.hint',
  },
  hatchDouble: {
    kind: 'flag', statusKey: 'hatchDouble',
    activeWhen: Object.freeze({ statusKey: 'hatchPatternType', values: Object.freeze([1, 2]) }),
    labelKey: 'status.tool_parameter.hatch_double',
    hintKey: 'status.tool_parameter.hatch_double.hint',
  },
  hatchStyle: {
    kind: 'number', statusKey: 'hatchStyle', step: 1,
    ...closedSet(0, 1, 2),
    labelKey: 'status.tool_parameter.hatch_style',
    hintKey: 'status.tool_parameter.hatch_style.hint',
  },
  // TEXT / MTEXT glyph height (ADR 0019 decision F.1). The host accepts any
  // finite value strictly above 0 (`ToolParameter::TextHeight`,
  // `finite_greater_than(0.0)`) and never clamps — the same rule the typed field
  // and the steps below enforce. A height is a real world-unit length, so the
  // value is `typed`: a 0.5 step reaches 2.0 and 3.0, but a drawing needs 1.75.
  // Both creation tools read the one session parameter, so both tools own the
  // row (see `TOOL_PARAMETERS`) and TEXT/MTEXT can never disagree about it.
  textHeight: {
    kind: 'number', statusKey: 'textHeight', step: 0.5,
    offered: finiteAbove(0), typed: true,
    labelKey: 'status.tool_parameter.text_height',
    hintKey: 'status.tool_parameter.text_height.hint',
  },
});

/** Which parameters belong to an active tool, and the command that gates them. */
export interface ToolParameterOwner {
  readonly command: string;
  readonly keys: readonly string[];
}

/** Every tool that owns typed parameters, keyed by its stable tool key. */
export type ToolParameterMap = Readonly<Record<string, ToolParameterOwner>>;

/**
 * Which parameters belong to which active tool (`cad_status.tool.key`), and the
 * command whose admission gates the control (`shell.canCommand`).
 *
 * Three kinds are reachable today, and each is reachable because the host really
 * projects the value while that tool runs: `draw.polygon` (its side count),
 * `draw.hatch` (its ten fill parameters; HATCH captures no points) and
 * `draw.text` / `draw.mtext` (the one shared text height). The other tools are
 * not implemented and the host projects no value for their parameters, so nothing
 * renders for them (a control appears only when the tool is active **and** its
 * value is projected). A later phase makes a tool live by adding one row here
 * (and, in the host, projecting its values). The remaining tool keys are the
 * anticipated `draw.<kind>` form of the frozen `CadTool.key` convention (ADR 0016
 * decisions B–E); the host must use the same key for the row to light up.
 */
export const TOOL_PARAMETERS: ToolParameterMap = Object.freeze({
  'draw.polygon': { command: 'POLYGON', keys: ['polygonSides'] },
  // The fill kind first, then the rows that kind scopes (the predefined pattern,
  // then the whole gradient), then the pattern-only rows and the style last. Only
  // one group is in effect at a time, but every row stays visible: a scoped row is
  // greyed with its reason rather than disappearing, so switching the fill kind
  // never moves the strip under the pointer.
  'draw.hatch': {
    command: 'HATCH',
    keys: [
      'hatchPatternType',
      'hatchPattern',
      'hatchGradientKind',
      'hatchGradientAngle',
      'hatchGradientColorA',
      'hatchGradientColorB',
      'hatchSpacing',
      'hatchAngle',
      'hatchDouble',
      'hatchStyle',
    ],
  },
  'draw.fillet': { command: 'FILLET', keys: ['filletRadius'] },
  'draw.chamfer': { command: 'CHAMFER', keys: ['chamferDistanceA', 'chamferDistanceB'] },
  'draw.align': { command: 'ALIGN', keys: ['alignScale'] },
  'draw.arrayrect': { command: 'ARRAYRECT', keys: ['arrayRows', 'arrayColumns', 'arrayRowSpacing', 'arrayColumnSpacing'] },
  'draw.arraypolar': { command: 'ARRAYPOLAR', keys: ['arrayItems', 'arrayFillAngle', 'arrayRotateItems'] },
  // TEXT and MTEXT create their glyphs with the **one** session `textHeight`
  // (ADR 0019 F.1; the host projects it for both kinds), so both tools own the
  // same single row. The gating command is the running tool's own.
  'draw.text': { command: 'TEXT', keys: ['textHeight'] },
  'draw.mtext': { command: 'MTEXT', keys: ['textHeight'] },
});

/** The live boolean of a flag `spec` from the active tool, or `null`. */
function projectedFlag(spec: FlagToolParameterSpec, tool: CadTool | null | undefined): boolean | null {
  const raw = tool?.params?.[spec.statusKey];
  return typeof raw === 'boolean' ? raw : null;
}

/** The live number of a numeric `spec` from the active tool, or `null`. */
function projectedNumber(spec: NumberToolParameterSpec, tool: CadTool | null | undefined): number | null {
  const raw = tool?.params?.[spec.statusKey];
  const projectable = spec.projectable ?? spec.offered;
  return typeof raw === 'number' && projectable(raw) ? raw : null;
}

/** A flag parameter's status-bar view. */
export interface ToolParameterFlagView {
  readonly kind: 'flag';
  readonly key: string;
  readonly value: boolean;
  /** The value a dispatch would set (always the opposite of `value`). */
  readonly next: boolean;
  readonly labelKey: string;
  readonly hintKey: string;
  readonly icon?: string;
  /**
   * Whether the row sits outside the sibling code that scopes it (`activeWhen`):
   * always `false` for a row without one. Scope, not capability — the toggle still
   * dispatches (the host accepts the write and simply ignores it), but the chip is
   * muted and its title says the current setting does not use it.
   */
  readonly inactive: boolean;
  readonly disabled: boolean;
}

/**
 * One named code of a closed-set parameter: the machine value the IPC carries
 * plus the catalog key of the name a user reads.
 *
 * The `value` is the integer the one parameter IPC sends, exactly as a step would
 * send it, so naming a code never changes what travels on the wire.
 */
export interface ToolParameterChoice {
  /** The integer code the host accepts — exactly the value the IPC carries. */
  readonly value: number;
  /** Catalog key of the code's name (present in both catalogs). */
  readonly labelKey: string;
  /**
   * Whether the row currently stands on this code: the projected status value plus
   * the bounded pending delta. A pick therefore shows at once, and the host's next
   * echo — or an explicit refusal, which drops the delta — corrects it.
   */
  readonly selected: boolean;
  /** Whether picking it would dispatch: command admission **and** the row's scope. */
  readonly disabled: boolean;
}

/** A numeric parameter's status-bar view: a bounded stepper, optionally a named chooser. */
export interface ToolParameterNumberView {
  readonly kind: 'number';
  readonly key: string;
  readonly value: number;
  readonly delta: number;
  readonly effective: number;
  readonly text: string;
  /** The increment the `-`/`+` buttons and the arrow keys use. */
  readonly step: number;
  /** Whether the value may also be typed in place (a real length), not only stepped. */
  readonly typed: boolean;
  /**
   * Whether the row is a **colour swatch** rather than a number to step (`spec.swatch`).
   * The wire value is still the integer ACI index this view carries; the row paints
   * that index from the shared palette and opens the shared colour chooser, which
   * can only hand back another index of the same palette.
   */
  readonly swatch: boolean;
  /**
   * The named closed set this row chooses among, in the host's own code order, or
   * `null` for a plain stepper. A named row is still a numeric row — the same
   * `choices` the `offered` predicate keeps, the same one IPC — so the chooser can
   * never offer a code a step could not, and the bounds below stay meaningful.
   */
  readonly choices: readonly ToolParameterChoice[] | null;
  /**
   * Whether the row sits outside the sibling code that scopes it (`activeWhen`):
   * always `false` for a row without one. This is **scope, not capability** — the
   * host would still accept a write — so it is reported separately from `disabled`
   * (which is command admission) and only mutes the chooser.
   */
  readonly inactive: boolean;
  readonly labelKey: string;
  readonly hintKey: string;
  readonly icon?: string;
  readonly disabled: boolean;
  readonly decrement: number;
  readonly increment: number;
  readonly canDecrement: boolean;
  readonly canIncrement: boolean;
}

/** One status-bar parameter view. */
export type ToolParameterView = ToolParameterFlagView | ToolParameterNumberView;

/** A bounded pending step the host has not echoed back yet. */
export interface ToolParameterPending {
  readonly key: string;
  /** The status value the delta is measured against. */
  readonly base: number;
  readonly delta: number;
}

/**
 * The status-bar views for every parameter of the currently active tool.
 *
 * Returns `[]` unless a tool is active with a map entry; each parameter yields a
 * numeric view (a bounded stepper, or a named chooser when its spec names the
 * codes) or a flag view, and a parameter whose value is absent (not
 * projected, `null`, or malformed/out of range) yields nothing at all — never an
 * invented default. `admitted(command)` is the shell's normal command admission
 * (`canCommand`), which already accounts for a pending command and a
 * loading/unavailable host or an unloaded vocabulary — the same gate the toggle
 * chips use.
 *
 * A numeric view's `value` (the *rendered* number) is always the real status
 * value and is never optimistically advanced. `effective` is `value + delta`,
 * where `delta` is the bounded pending magnitude the host has not echoed back yet
 * (see [`stepToolParameterPending`]); it only shifts the target the next step
 * builds on and the button bounds, so a rapid double-click composes into
 * `value + 2·step` while the displayed number still follows the status. A named
 * chooser marks the code at `effective`, so a pick reads as chosen at once and the
 * host's echo (or its refusal, which drops the delta) settles it.
 */
export function toolParameterViews(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean | ((command: string) => boolean),
  pending: ToolParameterPending | null = null,
): ToolParameterView[] {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return [];
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry) return [];
  const disabled = !(typeof admitted === 'function' ? admitted(entry.command) : admitted === true);
  const views: ToolParameterView[] = [];
  for (const key of entry.keys) {
    const spec = TOOL_PARAMETER_SPECS[key];
    if (!spec) continue;
    const view = parameterView(key, spec, tool, disabled, pending);
    if (view) views.push(view);
  }
  return views;
}

/**
 * Whether a row scoped by `activeWhen` applies under the current sibling code.
 *
 * An absent, `null` or non-numeric sibling value never claims the row applies: the
 * strip can only make a scope claim it can prove from `cad_status`, and the host
 * projects every parameter of an active tool together.
 */
function activeUnder(spec: ToolParameterSpec, tool: CadTool): boolean {
  const condition = spec.activeWhen;
  if (!condition) return true;
  const raw = tool.params?.[condition.statusKey];
  return typeof raw === 'number' && condition.values.includes(raw);
}

/**
 * The named choices of a closed set, or `null` when the row is a plain stepper.
 *
 * The codes come from the spec's own `choices` — the same list the `offered`
 * predicate keeps — so the chooser can never show a code the host would refuse,
 * and it lists them all: a code without a catalog name (a broken table) demotes
 * the whole row to the stepper instead of silently dropping a value the host
 * accepts. Every name is a catalog key, never copy: this module carries none.
 */
function choiceViews(
  spec: NumberToolParameterSpec,
  selectedCode: number,
  disabled: boolean,
  inactive: boolean,
): readonly ToolParameterChoice[] | null {
  const keys = spec.choiceLabelKeys;
  const codes = spec.choices;
  if (!keys || !codes) return null;
  const choices: ToolParameterChoice[] = [];
  for (const value of codes) {
    const labelKey = keys[value];
    if (typeof labelKey !== 'string' || labelKey.length === 0) return null;
    choices.push({
      value,
      labelKey,
      selected: value === selectedCode,
      disabled: disabled || inactive,
    });
  }
  return Object.freeze(choices);
}

function parameterView(
  key: string,
  spec: ToolParameterSpec,
  tool: CadTool,
  disabled: boolean,
  pending: ToolParameterPending | null,
): ToolParameterView | null {
  if (spec.kind === 'flag') {
    const value = projectedFlag(spec, tool);
    if (value === null) return null;
    return {
      kind: 'flag', key, value, next: !value,
      inactive: !activeUnder(spec, tool),
      labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    };
  }
  const value = projectedNumber(spec, tool);
  if (value === null) return null;
  const delta = pending && pending.key === key && pending.base === value ? pending.delta : 0;
  const effective = value + delta;
  const inactive = !activeUnder(spec, tool);
  return {
    kind: 'number', key, value, delta, effective, text: String(value),
    step: spec.step, typed: spec.typed === true, swatch: spec.swatch === true,
    choices: choiceViews(spec, effective, disabled, inactive),
    inactive,
    labelKey: spec.labelKey, hintKey: spec.hintKey, icon: spec.icon, disabled,
    decrement: effective - spec.step,
    increment: effective + spec.step,
    // A step is offered only while the *effective* target is a value the host
    // accepts and the pending magnitude is under the cap: it is disabled, never
    // clamped into a different value.
    canDecrement: spec.offered(effective - spec.step) && delta > -TOOL_PARAMETER_PENDING_CAP,
    canIncrement: spec.offered(effective + spec.step) && delta < TOOL_PARAMETER_PENDING_CAP,
  };
}

/**
 * Reconcile the bounded pending delta against the latest real status.
 *
 * Returns the same `pending` while the host has not yet caught up (the status
 * value still equals the value the delta is measured against, `base`), and
 * `null` — dropping the delta — as soon as the value changes, the tool
 * switches/cancels, the parameter leaves the active tool's map, the value becomes
 * absent/`null`/invalid, or the status carries an `error`. The error rule matters
 * because an in-queue host refusal (queue full, Viewer edge) surfaces through
 * `status.error` **without** changing the value: without it the delta would stay
 * stuck at its `base` and the next click would compose an absolute target up to
 * the cap away from the real value. Returning the same reference on an unchanged
 * poll lets the caller `setState` without triggering a re-render.
 */
export function reconcileToolParameterPending(
  pending: ToolParameterPending | null | undefined,
  status: Partial<CadStatus> | null | undefined,
): ToolParameterPending | null {
  if (!pending || typeof pending !== 'object') return null;
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  // A real host error (never an empty/absent string) drops the in-flight delta,
  // so the next step builds on the value the host actually holds.
  if (status?.error) return null;
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry || !entry.keys.includes(pending.key)) return null;
  const spec = TOOL_PARAMETER_SPECS[pending.key];
  if (!spec || spec.kind !== 'number') return null;
  const value = projectedNumber(spec, tool);
  if (value === null || value !== pending.base) return null;
  return pending;
}

/**
 * Fold one step into the pending delta, or `null` when there is nothing to track.
 *
 * `target` is the value a step dispatches (`effective ± step` from the view); the
 * new delta is `target - base`, measured against the current status value. A zero
 * delta (a step that lands back on the status value) and any delta beyond
 * [`TOOL_PARAMETER_PENDING_CAP`] collapse to `null`, so the cap can never be
 * exceeded even if a caller bypasses the view's disabled buttons.
 *
 * `pending` is the in-flight delta the caller's `target` was built on. Folding is
 * only meaningful onto the same parameter and the same `base` value: a mismatch
 * means the caller stepped off a stale target (the same class of drift the
 * reconcile guards against), so the step is refused rather than composed into an
 * absolute target the host never saw.
 */
export function stepToolParameterPending(
  pending: ToolParameterPending | null | undefined,
  status: Partial<CadStatus> | null | undefined,
  key: string,
  target: number,
): ToolParameterPending | null {
  const tool = status?.tool;
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  const entry = TOOL_PARAMETERS[tool.key];
  if (!entry || !entry.keys.includes(key)) return null;
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number') return null;
  const base = projectedNumber(spec, tool);
  if (base === null) return null;
  if (pending && (pending.key !== key || pending.base !== base)) return null;
  const delta = target - base;
  if (!Number.isFinite(delta) || delta === 0 || Math.abs(delta) > TOOL_PARAMETER_PENDING_CAP) return null;
  return { key, base, delta };
}

/** What one tool-parameter IPC did: queue admission, an explicit refusal, a failure. */
export type ToolParameterResult =
  | { readonly kind: 'accepted' }
  | { readonly kind: 'refused'; readonly error: string }
  | { readonly kind: 'failed'; readonly error: string };

/**
 * Ask the host to set one numeric tool parameter.
 *
 * `cad_set_tool_parameter` is the one IPC for every parameter. A
 * `{accepted:true}` result is **queue admission**, exactly like `cad_command`:
 * the next real `cad_status` carries the new value, so the caller must not
 * optimistically mirror it. A value the host would refuse is refused here before
 * IPC (the host refuses it too rather than clamping); a pre-queue refusal rejects
 * with a string, returned as an explicit `failed` — never a silent success or a
 * fabricated local value.
 */
export async function setToolParameterNumber(
  invoke: Invoke,
  key: string,
  number: number,
): Promise<ToolParameterResult> {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number' || !spec.offered(number)) {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_OUT_OF_RANGE' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, number });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * Ask the host to set one flag tool parameter.
 *
 * The caller passes the **opposite** of the value it read from the status (never
 * a fabricated one). Admission semantics match [`setToolParameterNumber`]: the
 * result is queue admission, and a refusal is explicit.
 */
export async function setToolParameterFlag(
  invoke: Invoke,
  key: string,
  flag: boolean,
): Promise<ToolParameterResult> {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'flag' || typeof flag !== 'boolean') {
    return { kind: 'refused', error: 'CAD_TOOL_PARAMETER_KIND' };
  }
  try {
    const response = await invoke(TOOL_PARAMETER_COMMAND, { parameter: key, flag });
    if (!response || (response as { accepted?: unknown }).accepted !== true) throw Error('CAD_TOOL_PARAMETER_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

/**
 * The catalog key of the refusal shown for a typed value the host would refuse.
 *
 * It is a catalog key, not copy: the strip translates it with the parameter's own
 * localized name, so the message can never drift from the label it is about.
 */
export const TOOL_PARAMETER_INVALID_KEY = 'status.tool_parameter.invalid';

/**
 * The catalog key of the reason a scoped row is inert right now.
 *
 * It is a catalog key, not copy: the strip prefixes it to the row's own hint, so a
 * greyed chooser says *why* it is greyed without a second hand-written sentence
 * per parameter. Scope is not capability — the host would still accept the write —
 * so the sentence talks about what the current setting uses, never about support.
 */
export const TOOL_PARAMETER_INACTIVE_KEY = 'status.tool_parameter.inactive';

/**
 * The local draft of one typed numeric parameter.
 *
 * A draft is **text the user is typing**, never a copy of the host value: while
 * `text` is `null` the field renders the live `cad_status` number, and once the
 * user types, only what they typed is shown — a poll landing mid-edit can never
 * overwrite it. `messageKey` is the catalog refusal the field is currently
 * showing, and it is cleared by the next keystroke and by Escape.
 */
export interface ToolParameterDraft {
  /** What the field shows instead of the host value, or `null` when untouched. */
  readonly text: string | null;
  /** Catalog key of the refusal being shown, or `null` when there is none. */
  readonly messageKey: string | null;
}

/** No draft: the field shows the host value and no refusal. */
export const NO_TOOL_PARAMETER_DRAFT: ToolParameterDraft = Object.freeze({ text: null, messageKey: null });

/** Fold one keystroke into the draft; typing clears the previous refusal. */
export function typeToolParameterDraft(_draft: ToolParameterDraft | null | undefined, text: string): ToolParameterDraft {
  return { text: typeof text === 'string' ? text : '', messageKey: null };
}

/**
 * Drop the draft: Escape (and a `-`/`+` step, which is an absolute target built
 * from the status) puts the host value back in the field.
 */
export function cancelToolParameterDraft(): ToolParameterDraft {
  return NO_TOOL_PARAMETER_DRAFT;
}

/** What one commit of the draft decided. */
export type ToolParameterDraftCommit =
  /** Nothing was typed: no write, no message — an untouched field's blur. */
  | { readonly kind: 'idle' }
  /** Refused before dispatch: the draft keeps the text and names the message. */
  | { readonly kind: 'refused'; readonly draft: ToolParameterDraft }
  /** Accepted by the parameter's own guard: the caller dispatches `number`. */
  | { readonly kind: 'write'; readonly number: number; readonly draft: ToolParameterDraft };

// A plain decimal (optionally signed, optionally with an exponent) and nothing
// else. This is the field's **syntax**, not its range: `1.75` and `2` parse, an
// empty field and `abc` do not, and the range stays exactly the host's own
// `offered` predicate below — no stricter and no looser bound is invented here.
const DECIMAL_TEXT = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;

/**
 * Decide what one commit of a typed value does — never a silent clamp.
 *
 * `0`, a negative number, `NaN`, an empty field, a non-numeric string and any
 * value the parameter's own `offered` predicate refuses all come back `refused`
 * with [`TOOL_PARAMETER_INVALID_KEY`], so the caller shows the catalog message
 * and **nothing is dispatched**. A value the host accepts (including a decimal
 * such as `1.75`) is returned **verbatim** as a number: no rounding, no
 * re-derivation from the steps.
 */
export function commitToolParameterDraft(
  key: string,
  draft: ToolParameterDraft | null | undefined,
): ToolParameterDraftCommit {
  const raw = typeof draft?.text === 'string' ? draft.text.trim() : null;
  if (raw === null) return { kind: 'idle' };
  const spec = TOOL_PARAMETER_SPECS[key];
  const number = DECIMAL_TEXT.test(raw) ? Number(raw) : Number.NaN;
  if (!spec || spec.kind !== 'number' || !spec.offered(number)) {
    return { kind: 'refused', draft: { text: draft?.text ?? raw, messageKey: TOOL_PARAMETER_INVALID_KEY } };
  }
  return { kind: 'write', number, draft: NO_TOOL_PARAMETER_DRAFT };
}

/**
 * The value one arrow-key step from the field offers, or `null` when the step is
 * not a value the host accepts.
 *
 * The step starts from the draft when the field holds a usable number and from
 * `base` (the view's effective target, the same one the `-`/`+` buttons step
 * from) otherwise, so an arrow key never composes off half-typed text. A target
 * outside the parameter's range is simply not offered — refused, never clamped.
 */
export function stepToolParameterDraft(
  key: string,
  draft: ToolParameterDraft | null | undefined,
  base: number,
  direction: 1 | -1,
): number | null {
  const spec = TOOL_PARAMETER_SPECS[key];
  if (!spec || spec.kind !== 'number') return null;
  const typed = commitToolParameterDraft(key, draft);
  const from = typed.kind === 'write' ? typed.number : base;
  const target = from + direction * spec.step;
  return Number.isFinite(target) && spec.offered(target) ? target : null;
}
