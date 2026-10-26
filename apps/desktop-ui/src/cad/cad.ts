import { MTEXT_STEP_PROMPT_KEY, MTEXT_STEP_PROMPT_KEYS } from '../tools/text.ts';
import { TEXT_PICK_PROMPT_KEY, TEXT_STEP_PROMPT_KEY, textStepPromptKey, toolAwaitingText } from '../tools/text.ts';
import { OPEN_ENDED_PROMPTS } from '../tools/tool-open-ended.ts';
import type { CadStatus, CadTool } from './cad-status.ts';


/** A status-flag chip: whether it is on, and whether its command is admitted. */
export interface StatusToggleView {
  readonly on: boolean;
  readonly disabled: boolean;
}

/** The grid-snap chip: its switch plus the real spacing the host reports. */
export interface GridSnapToggleView extends StatusToggleView {
  readonly spacing: number | null;
}

/** The single router command that flips idle hover preview. */
export const HOVER_PREVIEW_COMMAND = 'HOVERPREVIEW';

/**
 * Status-bar view for the idle hover-preview toggle.
 *
 * The server's `hoverPreview` flag is the single source of truth. A payload
 * without the field (older host) yields `null`, so the caller renders no chip at
 * all instead of guessing `false`. `admitted` is the shell's normal command
 * admission (`canCommand`), which already accounts for a pending command and a
 * loading/unavailable host.
 */
export function hoverPreviewToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.hoverPreview;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchHoverPreview(runCommand: (command: string) => void): void {
  runCommand(HOVER_PREVIEW_COMMAND);
}

/** The single router command that flips the object-snap master switch (F3). */
export const OBJECT_SNAP_COMMAND = 'OSNAP';

/**
 * Status-bar view for the object-snap master switch (ADR 0014 A.4).
 *
 * The server's `snapEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not the persisted
 * `view.overlays.snapHints` preference. A payload without the field (older host)
 * yields `null`, so the caller renders no chip at all instead of guessing
 * `false`. `admitted` is the shell's normal command admission (`canCommand`),
 * which already accounts for a pending command and a loading/unavailable host.
 */
export function objectSnapToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.snapEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchObjectSnap(runCommand: (command: string) => void): void {
  runCommand(OBJECT_SNAP_COMMAND);
}

/** The single router command that flips the display grid (`GRID`, F7). */
export const GRID_COMMAND = 'GRID';

/**
 * Status-bar view for the display-grid switch (`GRID`, F7).
 *
 * The server's `gridEnabled` flag is the single source of truth: it is the
 * session's live switch. An on chip does **not** by itself mean the grid is
 * drawn — the host also requires the persisted `view.overlays.grid` preference,
 * so the two gates compose with AND. This is the display grid only; grid snap
 * (`SNAP`/F9) is the separate [`gridSnapToggle`]. A payload without the field
 * yields `null`, so the caller renders no chip at all instead of guessing
 * `false`. `admitted` is the shell's normal command admission (`canCommand`).
 */
export function gridToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.gridEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchGrid(runCommand: (command: string) => void): void {
  runCommand(GRID_COMMAND);
}

/** The single router command that flips grid snap (`SNAP`, F9). */
export const GRID_SNAP_COMMAND = 'SNAP';

/**
 * Status-bar view for the grid-snap switch (`SNAP`, F9).
 *
 * The server's `snapGridEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not the display grid. While on,
 * the host quantises a captured point to the nearest multiple of the session
 * `snapSpacing` in X and Y (never Z) before the `ORTHO` constraint; `spacing` is
 * that real projected value, present only when it is a usable number (> 0), so
 * the chip never advertises a spacing the host did not report. A payload without
 * the field (older host) yields `null`, so the caller renders no chip at all
 * instead of guessing `false`. `admitted` is the shell's normal command
 * admission (`canCommand`).
 */
export function gridSnapToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): GridSnapToggleView | null {
  const on = status?.snapGridEnabled;
  if (typeof on !== 'boolean') return null;
  const spacing = status?.snapSpacing;
  return {
    on,
    spacing: typeof spacing === 'number' && Number.isFinite(spacing) && spacing > 0 ? spacing : null,
    disabled: admitted !== true,
  };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchGridSnap(runCommand: (command: string) => void): void {
  runCommand(GRID_SNAP_COMMAND);
}

/** The single router command that flips the ORTHO drafting constraint (F8). */
export const ORTHO_COMMAND = 'ORTHO';

/**
 * Status-bar view for the orthographic drafting constraint (`ORTHO`, F8).
 *
 * The server's `orthoEnabled` flag is the single source of truth: it is the
 * session's live switch, not a local mirror and not a persisted preference. While
 * on, the host constrains the next point entry (and the matching rubber band) to a
 * horizontal or vertical direction from the tool's previous point; it is not grid
 * snap (`SNAP`/F9) and it never moves a point in Z. A payload without the field
 * (older host) yields `null`, so the caller renders no chip at all instead of
 * guessing `false`. `admitted` is the shell's normal command admission
 * (`canCommand`).
 */
export function orthoToggle(
  status: Partial<CadStatus> | null | undefined,
  admitted: boolean,
): StatusToggleView | null {
  const on = status?.orthoEnabled;
  if (typeof on !== 'boolean') return null;
  return { on, disabled: admitted !== true };
}

/** Dispatch the toggle as the one command the router accepts. */
export function dispatchOrtho(runCommand: (command: string) => void): void {
  runCommand(ORTHO_COMMAND);
}

/**
 * The global drafting-aid accelerators: function key -> router command.
 *
 * A **name** table, not a second key handler. The shell has exactly one global
 * `keydown` listener (`App.jsx`), and it is the only place a function key may be
 * claimed: it resolves the key through [`draftingAcceleratorCommand`] and
 * submits the returned command through the ordinary funnel (`submitCommand`) —
 * the same path every chip uses. `F7` is AutoCAD's own key for the display grid,
 * `F3` its object-snap key, `F8` its orthographic-constraint key and `F9` its
 * grid-snap key.
 */
export const DRAFTING_ACCELERATORS: Readonly<Record<string, string>> = Object.freeze({
  F3: 'OSNAP', F7: 'GRID', F8: 'ORTHO', F9: 'SNAP',
});

/** The router command a drafting-aid function key means, or `null`. */
export function draftingAcceleratorCommand(key: unknown): string | null {
  if (typeof key !== 'string') return null;
  return DRAFTING_ACCELERATORS[key] ?? null;
}


/**
 * The catalog key for ID's single pick.
 *
 * ID reports the coordinates of the point the user picks, so its one step is a
 * plain "specify a point" prompt — `tool.prompt.point`, the catalog's own point
 * sentence. It borrowed the dimension feature-location copy only while that key
 * did not exist yet; a dimension prompt would name the wrong operation.
 */
const ID_POINT_PROMPT_KEY = 'tool.prompt.point';

/**
 * Per-step capture prompts for tools whose picks have names (ADR 0017 P5c–P5e).
 *
 * The dimension commands capture named picks — the linear/aligned pair three
 * (two extension-line origins, then the dimension-line location), the radial
 * pair two (an arc/circle entity pick, then the dimension-line location),
 * DIMCENTER one (an arc/circle entity pick), DIMANGULAR three (two straight-line
 * entity picks, then the arc location) and DIMORDINATE two (a feature point, then
 * the leader endpoint) — and a bare point count ("1 point") does not tell the user
 * *what* to pick next. This table names each pick; the step is derived from the
 * host's real `tool.key` and `tool.points` (the captured count), never inferred
 * from the pointer stream. Every other tool has no entry, so it renders no prompt
 * at all and its chip is unchanged.
 *
 * `location_linear` vs `location_aligned` differ on purpose: for DIMLINEAR the
 * location pick also decides horizontal vs vertical, which the user must know
 * before clicking.
 *
 * The radial pair's second pick is deliberately **not** described as positioning
 * the text: the location pick only sets the leader's direction from the centre,
 * and the synthesis derives the text placement itself. Its prompt asks only for
 * the "dimension location" so it never claims the user places the text.
 *
 * DIMANGULAR is the **two-line** form only: its first two steps are line entity
 * picks, never a vertex point. The two lines' infinite intersection is the vertex
 * and the third pick is the arc location, which selects which of the four angles
 * is measured — the backend's rule is "the sector the location falls in". The copy
 * asks for the arc location and makes no quadrant-picker claim, because no such
 * picker exists. The three-point (vertex + two side points) construction is
 * supported by the command payload only and has **no interactive capture**, so no
 * prompt may imply a vertex-first workflow.
 *
 * DIMORDINATE's two steps are free points (no entity pick) and the interactive
 * tool never forces an axis: the backend derives it from the documented auto rule
 * (a greater Y difference measures the X ordinate; the unstated equal case is our
 * declared choice of Y). The prompts name the two points only, so they promise no
 * axis control the tool does not offer.
 *
 * MLEADER's two steps are free points too — the arrowhead position, then the
 * landing point — followed by the same terminal text step TEXT uses. Its prompts
 * name those two picks and nothing else: the command has **no `Options`
 * sub-prompt** (leader type, content type, max points, angular constraint), so no
 * step copy or control may imply one.
 *
 * Only the **named picks** live here, as an ordered list (a step is chosen by the
 * captured count). A completed capture's ready prompt is not positional, so it is
 * resolved from [`TOOL_READY_PROMPTS`] below.
 */
export const TOOL_STEP_PROMPTS: Readonly<Record<string, readonly string[]>> = Object.freeze({
  'draw.textedit': ['tool.prompt.textedit.select_entity'],
  'draw.dimlinear': [
    'tool.prompt.dim.origin_first',
    'tool.prompt.dim.origin_second',
    'tool.prompt.dim.location_linear',
  ],
  'draw.dimaligned': [
    'tool.prompt.dim.origin_first',
    'tool.prompt.dim.origin_second',
    'tool.prompt.dim.location_aligned',
  ],
  'draw.dimradius': [
    'tool.prompt.dim.select_arc',
    'tool.prompt.dim.location_radius',
  ],
  'draw.dimdiameter': [
    'tool.prompt.dim.select_arc',
    'tool.prompt.dim.location_diameter',
  ],
  'draw.dimcenter': [
    'tool.prompt.dim.select_arc',
  ],
  'draw.dimangular': [
    'tool.prompt.dim.select_line',
    'tool.prompt.dim.select_second_line',
    'tool.prompt.dim.location_angular',
  ],
  'draw.dimordinate': [
    'tool.prompt.dim.feature',
    'tool.prompt.dim.leader_endpoint',
  ],
  // TEXT captures one point (its insertion point) and then takes its text. Only
  // the point is a **named pick** here; the text step is not positional (it has
  // no captured count of its own) and is named from the host's `awaiting_text`
  // flag by [`toolStepPromptKey`] — never by counting points.
  'draw.text': [
    TEXT_PICK_PROMPT_KEY,
  ],
  // MTEXT (2026-10-26) captures two named corners: the first is the insertion
  // point, the opposite one only defines the wrap width. Its terminal text step
  // is named from `awaiting_text` by [`textStepPromptKey`], never by a count.
  'draw.mtext': MTEXT_STEP_PROMPT_KEYS,
  // MLEADER (ADR 0019 decision F.2) captures exactly two points — the arrowhead
  // position, then the landing point — and then the **same** terminal text step
  // TEXT uses. Only those two picks belong here: the text step is named from the
  // host's `awaiting_text` flag by [`toolStepPromptKey`], never from a point
  // count, so this list must stay at two entries. The two names are
  // MLEADER-specific because a leader's first pick is its arrowhead, not a
  // generic "point", and its second is the landing the text hangs from.
  'draw.mleader': [
    'tool.prompt.mleader.arrow',
    'tool.prompt.mleader.landing',
  ],
  // ID (2026-10-26) is a **single-point** capture that commits itself: one pick,
  // then the host reports the coordinates. It therefore has exactly one step and
  // never reaches the ready state in practice (the capture is complete the moment
  // the pick lands), so the list must stay at one entry. The wording is the
  // catalog's own point-pick sentence (`tool.prompt.point`).
  'measure.id': [
    ID_POINT_PROMPT_KEY,
  ],
});

/**
 * Per-tool "ready" prompt override, keyed by the active tool key (ADR 0017 P5d).
 *
 * The named-pick table above must stay an **ordered list** (a step is chosen by
 * the captured count), so the ready prompt — which is not positional — lives in
 * this parallel table. Once the exact complement is captured the tool is ready
 * and waits for Enter; the shared default `tool.prompt.dim.ready` says a
 * **dimension** is defined, which is correct for the real dimension commands.
 * DIMCENTER is the one exception: it emits plain line geometry (a centre mark or
 * centrelines), not a dimension entity, so it must not claim a dimension was
 * defined and gets `tool.prompt.dim.center_ready` instead. DIMANGULAR and
 * DIMORDINATE both commit real `DIMENSION` entities, so they deliberately have no
 * entry here and fall through to the shared dimension ready copy; reusing the
 * centre-mark copy for them would be false. A tool not listed here falls back to
 * the shared dimension ready prompt.
 */
export const TOOL_READY_PROMPTS: Readonly<Record<string, string>> = Object.freeze({
  'draw.dimcenter': 'tool.prompt.dim.center_ready',
  // TEXT's complement is its point plus its text, so "ready" is only reached
  // between the two states: the shared dimension copy ("Dimension defined; press
  // Enter to confirm") would be false for a text command, and the next step is
  // always the text. This names that step; the **input mode** still comes from
  // the host's `awaiting_text` alone (see [`toolStepPromptKey`]), so a race where
  // the point is counted but the flag has not arrived yet never opens the text
  // field — it only names what comes next.
  'draw.text': TEXT_STEP_PROMPT_KEY,
  // MTEXT's complement is its two corners plus its text, so it has the same
  // "the next step is the text" override, with MTEXT's own multiline wording. The
  // input mode still comes from the host's `awaiting_text` alone.
  'draw.mtext': MTEXT_STEP_PROMPT_KEY,
  // MLEADER's complement is its two points plus its text. Once the landing pick
  // is captured the capture is complete but the host may not have raised
  // `awaiting_text` yet, and the shared dimension copy ("Dimension defined") would
  // be false for a leader. This names the step that really comes next — the text
  // — without opening it: the **input mode** still comes from the host flag alone
  // (see [`toolStepPromptKey`]), so the field never appears before the host says
  // so.
  'draw.mleader': 'tool.prompt.mleader.type_text',
  // ID's capture is complete after its single pick, so it commits itself and the
  // shared dimension ready copy ("Dimension defined; press Enter to confirm")
  // would be false twice over — nothing is dimensioned and there is nothing left
  // to confirm. The generic "Enter to confirm" sentence is the honest override for
  // the one state a race can still expose (the point counted before the host
  // closed the tool).
  'measure.id': 'tool.prompt.confirm',
});

/** A pre-selection tool's two prompts: waiting for a selection, and ready. */
export interface PreselectPrompt {
  readonly idle: string;
  readonly ready: string;
}

/**
 * Pre-selection tools: the boundary already exists before the command starts, so
 * the chip has no pick to name and cannot use a point index (ADR 0018 decision D,
 * P6b).
 *
 * HATCH's point arity is the zero-point exact complement — `tool.points` stays 0
 * from the moment the tool starts to the moment it commits — so an entry in
 * [`TOOL_STEP_PROMPTS`] could not express it: an empty step list makes that
 * derivation report "ready" before anything is selected, and any non-empty list
 * would invent a pick the tool never takes. The host's real `tool.ready`
 * (`DrawPreview::can_confirm`, true only for a non-empty selection) is the
 * authority, so idle and ready are the only two states. Keeping this in its own
 * table also guarantees a pre-selection tool can never inherit the shared
 * dimension ready copy.
 */
export const PRESELECT_PROMPTS: Readonly<Record<string, PreselectPrompt>> = Object.freeze({
  'draw.hatch': Object.freeze({
    idle: 'tool.prompt.hatch.select_boundary',
    ready: 'tool.prompt.hatch.ready',
  }),
});

/** The shared ready prompt for a capture that produced a dimension entity. */
const DEFAULT_READY_PROMPT: string = 'tool.prompt.dim.ready';

/**
 * The catalog key for the next expected pick of the active `tool`, or `null`.
 *
 * A tool the host reports as `awaiting_text` names its **text step** first — that
 * step is terminal and is not a pick, so no point count can express it; the flag
 * is the single source of truth ([`toolAwaitingText`]).
 *
 * A pre-selection tool (see [`PRESELECT_PROMPTS`]) captures no picks at all, so
 * its chip is chosen by the host's readiness instead: the "select the boundary
 * objects first" copy while it waits for a selection, the "boundary ready" copy
 * once the host reports it can be confirmed. `ready` must be the literal `true`;
 * a missing or malformed readiness never claims the selection is sufficient.
 *
 * `null` for a tool with no named steps and no pre-selection prompt (every other
 * existing tool) so the chip stays exactly as it was. Once the exact complement
 * is captured the capture is ready and waits for Enter, so the ready prompt names
 * that instead of repeating the last pick (a per-tool override from
 * [`TOOL_READY_PROMPTS`], else the shared dimension ready prompt). The point
 * count is read as the host's real value; a malformed one is treated as "no
 * points yet" rather than a fabricated later step.
 *
 * An open-ended tool ([`OPEN_ENDED_PROMPTS`]) has no fixed complement, so the
 * count only distinguishes "base dimension still missing" from "capturing
 * points". The base pick is the tool's first captured step — the same shape
 * DIMRADIUS/DIMCENTER already have — and the command refuses a capture with zero
 * measured points, so `points <= 1` always names the point step and the host's
 * `ready` is what turns the chip into the ready copy. No step count is ever
 * assumed: the point prompt repeats for as many points as the user picks.
 */
export function toolStepPromptKey(tool: CadTool | null | undefined): string | null {
  if (!tool || typeof tool !== 'object' || typeof tool.key !== 'string') return null;
  // The host's `awaiting_text` names the text step outright (ADR 0019 A.2). It is
  // checked before any count-derived step: the text step is not a pick, and the
  // flag is the only source of truth for it. The wording is per tool
  // ([`textStepPromptKey`]) so MTEXT's multiline copy is never TEXT's.
  if (toolAwaitingText(tool)) return textStepPromptKey(tool);
  const preselect = PRESELECT_PROMPTS[tool.key];
  if (preselect) return tool.ready === true ? preselect.ready : preselect.idle;
  const points = Number.isSafeInteger(tool.points) && tool.points >= 0 ? tool.points : 0;
  const openEnded = OPEN_ENDED_PROMPTS[tool.key];
  if (openEnded) {
    if (points === 0) return openEnded.select;
    // Readiness never wins without a measured point: a capture with only the base
    // pick is refused by the command, so claiming "ready" there would be false.
    if (points > 1 && tool.ready === true) return DEFAULT_READY_PROMPT;
    return openEnded.point;
  }
  const steps = TOOL_STEP_PROMPTS[tool.key];
  if (!steps) return null;
  if (points < steps.length) return steps[points];
  return TOOL_READY_PROMPTS[tool.key] ?? DEFAULT_READY_PROMPT;
}

// The snap engine's stable machine keys (`cad_measure::SnapKind::as_str`) and
// their catalog labels. A key outside this set is rendered as nothing rather
// than a guessed label.
const SNAP_KIND_LABELS: Readonly<Record<string, { readonly labelKey: string }>> = {
  endpoint: { labelKey: 'snap.kind.endpoint' },
  midpoint: { labelKey: 'snap.kind.midpoint' },
  center: { labelKey: 'snap.kind.center' },
  quadrant: { labelKey: 'snap.kind.quadrant' },
  perpendicular: { labelKey: 'snap.kind.perpendicular' },
  intersection: { labelKey: 'snap.kind.intersection' },
};

/**
 * The catalog label key for the current object-snap candidate kind, or `null`.
 *
 * The server's `snapKind` is the single source of truth (ADR 0014 A.7): it is
 * the current best candidate's machine kind while a point prompt is active, and
 * `null` otherwise. An absent/`null`/unknown kind yields `null`, so the caller
 * renders nothing extra instead of a placeholder chip or a guessed label.
 */
export function snapKindLabelKey(status: Partial<CadStatus> | null | undefined): string | null {
  const kind = status?.snapKind;
  if (typeof kind !== 'string') return null;
  return SNAP_KIND_LABELS[kind]?.labelKey ?? null;
}
