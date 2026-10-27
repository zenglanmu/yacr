// The active tool's typed-parameter contract (ADR 0016 decision A): the control
// spec table mirrored from the frozen host table, and which parameters each tool
// owns.
//
// Split verbatim out of `tool-parameters.ts` (which owns the status-bar views, the
// bounded pending delta, the typed-entry draft and the one IPC) so neither module
// approaches the 1000-line code gate. Plain sibling module, not a barrel:
// importers name it directly. The rows are the host mirror — the view code reads
// them and never the other way round, so this module imports nothing.

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
   * How a **named** closed set is laid out on the strip.
   *
   * `'segments'` (the default) is one row of segments, which is what every set so
   * far uses. `'grid'` is a compact chip that opens the same names as a grid —
   * reserved for a set whose codes **are** a spatial arrangement, where nine
   * segments with readable names would take roughly 600 px of status strip and
   * push the drafting aids off a narrow window. MTEXT's nine attachment codes are
   * AutoCAD's 3×3 anchor grid, so the grid lays them out in the three rows and
   * three columns the codes mean: the arrangement itself is the diagram, and the
   * chip still sends the same integer a segment would.
   *
   * A row without `choiceLabelKeys` has no named set, so this field does nothing.
   */
  readonly choiceLayout?: 'segments' | 'grid';
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
 * - `choiceLayout` — `'segments'` (the default named chooser) or `'grid'` (the
 *   compact grid MTEXT's 3×3 attachment codes open from one chip). Presentation
 *   only: the wire still carries the integer code;
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
  // TEXT / MTEXT creation rotation, in **degrees** (the unit AutoCAD prompts
  // with, exactly like the two HATCH angle rows). The host accepts any finite
  // value and never wraps it — `-30` stays `-30`, and a full turn is not folded
  // into `0` — so `offered` is plain finiteness and both steps stay available.
  // The degrees are converted to radians exactly once by the host, never here.
  // Both creation tools own the row (see `TOOL_PARAMETERS`).
  //
  // It steps by 15° like the other angle rows and is also `typed`: a rotation is
  // routinely a value no fixed increment reaches (`12`, `22.5`, `-33.75`), and a
  // typed value travels the same one IPC through the same `offered` guard.
  textRotation: {
    kind: 'number', statusKey: 'textRotation', step: 15,
    offered: finiteAny(), typed: true,
    labelKey: 'status.tool_parameter.text_rotation',
    hintKey: 'status.tool_parameter.text_rotation.hint',
  },
  // MTEXT attachment: AutoCAD's **nine** attachment codes (DXF group 71), a
  // closed set `1..=9` — `1..=3` are the top row (left/centre/right), `4..=6` the
  // middle row and `7..=9` the bottom row. Unlike a pattern index, the code is a
  // position, so the nine codes are read **by name** and laid out as the 3×3 grid
  // they describe (`choiceLayout: 'grid'`): a name plus its place in the grid is
  // the whole meaning, and a raw 1..9 stepper would be a number nobody decodes.
  // The wire keeps the integer, exactly as the host's `validate` demands.
  //
  // MTEXT is the only owner: TEXT stays single-line at its insertion point and
  // has no attachment, so this row never appears for `draw.text`. MTEXT's line
  // spacing, columns, background fill and drawing direction are NOT IMPLEMENTED
  // and therefore have no parameter (and no control) here.
  mtextAttachment: {
    kind: 'number', statusKey: 'mtextAttachment', step: 1,
    ...closedSet(1, 2, 3, 4, 5, 6, 7, 8, 9),
    choiceLabelKeys: Object.freeze({
      1: 'status.tool_parameter.mtext_attachment.top_left',
      2: 'status.tool_parameter.mtext_attachment.top_center',
      3: 'status.tool_parameter.mtext_attachment.top_right',
      4: 'status.tool_parameter.mtext_attachment.middle_left',
      5: 'status.tool_parameter.mtext_attachment.middle_center',
      6: 'status.tool_parameter.mtext_attachment.middle_right',
      7: 'status.tool_parameter.mtext_attachment.bottom_left',
      8: 'status.tool_parameter.mtext_attachment.bottom_center',
      9: 'status.tool_parameter.mtext_attachment.bottom_right',
    }),
    choiceLayout: 'grid',
    labelKey: 'status.tool_parameter.mtext_attachment',
    hintKey: 'status.tool_parameter.mtext_attachment.hint',
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
 * `draw.text` / `draw.mtext` (the one shared text height and the one shared
 * creation rotation, plus the MTEXT-only attachment). The other tools are
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
  // (ADR 0019 F.1; the host projects it for both kinds) and read the **one**
  // `textRotation` (degrees) at creation, so both tools own those two rows. MTEXT
  // alone owns `mtextAttachment`: TEXT is single-line at its insertion point and
  // has no attachment code. The gating command is the running tool's own, and the
  // order is the order the strip shows (height, rotation, then the MTEXT-only
  // attachment).
  'draw.text': { command: 'TEXT', keys: ['textHeight', 'textRotation'] },
  'draw.mtext': { command: 'MTEXT', keys: ['textHeight', 'textRotation', 'mtextAttachment'] },
});
