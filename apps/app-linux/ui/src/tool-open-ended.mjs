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
export const OPEN_ENDED_PROMPTS = Object.freeze({
  'draw.dimbaseline': Object.freeze({
    select: 'tool.prompt.dim.select_base',
    point: 'tool.prompt.dim.baseline_point',
  }),
  'draw.dimcontinue': Object.freeze({
    select: 'tool.prompt.dim.select_base',
    point: 'tool.prompt.dim.continue_point',
  }),
});
