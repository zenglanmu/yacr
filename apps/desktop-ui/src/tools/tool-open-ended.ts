// Open-ended dimension prompts (`DIMBASELINE`/`DIMCONTINUE`, ADR 0017 decision C).
//
// Both commands start by picking an existing linear dimension as the **base** and
// then accept any number of measured points until Enter. The fixed-arity table in
// `cad.mjs` (`TOOL_STEP_PROMPTS`) indexes its list by the captured count, so it
// cannot express that shape: past the first free point the index runs off the end
// and would claim the capture is ready, and a list long enough to cover every
// accepted point would invent steps. The real shape is "one base pick, then the
// same point prompt repeated", so it lives in this module (kept out of `cad.mjs`,
// which is at the file-size ceiling).
//
// Keys only: the catalogs stay the single authority for the copy, and readiness
// is the host's `ready` (see `toolStepPromptKey` in `cad.mjs`) — the same signal
// the PLINE chip reads, never a hard-coded number of steps.

/** The two catalog keys one open-ended tool uses: its base pick and its repeat. */
export interface OpenEndedPrompts {
  /** Prompt for the base dimension/entity pick. */
  readonly select: string;
  /** Prompt repeated for every accepted point. */
  readonly point: string;
}

/** Prompt keys by tool key (`cad.mjs`'s `tool.key`). */
export const OPEN_ENDED_PROMPTS: Readonly<Record<string, OpenEndedPrompts>> = Object.freeze({
  'draw.dimbaseline': Object.freeze({
    select: 'tool.prompt.dim.select_base',
    point: 'tool.prompt.dim.baseline_point',
  }),
  'draw.dimcontinue': Object.freeze({
    select: 'tool.prompt.dim.select_base',
    point: 'tool.prompt.dim.continue_point',
  }),
  // WIPEOUT (2026-11-07) is open-ended **without** a base pick: every point is a
  // boundary vertex, and Enter finishes the ring. `select` therefore names the
  // first vertex (it is not a base point) and `point` repeats for the next ones.
  // The three-vertex minimum is the host's `ready` (`AtLeast(3)`), never a step
  // count re-derived here; the ready copy is the tool's own override in
  // `TOOL_READY_PROMPTS`, because the shared dimension sentence would be false.
  'draw.wipeout': Object.freeze({
    select: 'tool.prompt.wipeout.first',
    point: 'tool.prompt.wipeout.next',
  }),
});
