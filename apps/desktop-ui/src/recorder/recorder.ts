// The Action Recorder's wire contract and pure normalisation (ADR-0056).
//
// The host owns the recorder state; this module only reads the
// `cad_action_recorder` host read, normalises its answer, and builds the typed
// command lines the panel sends through `cad_command`. There is no file picker:
// a macro destination is the user's typed path in the panel's own text field,
// and the host's `actstop.path` / `unsupported-runtime: action-recorder`
// refusals are shown verbatim.
//
// Wire shape (`cad_action_recorder`):
//
// ```jsonc
// {
//   "recording": { "name": "door", "path": "/tmp/door.json", "steps": 2 } | null,
//   "macros": [{ "name": "door", "path": "/tmp/door.json", "steps": 4 }],
//   "lastMacro": "door" | null,
//   "playback": { "name": "door", "nextStep": 2, "total": 4 } | null
// }
// ```

/** The host-read command (shared by both transports). */
export const ACTION_RECORDER_COMMAND = 'cad_action_recorder';
/** The one command funnel every recorder write rides. */
export const ACTION_COMMAND = 'cad_command';

/** One normalised recording buffer. */
export interface RecorderRecording {
  readonly name: string;
  readonly path: string | null;
  readonly steps: number;
}

/** One normalised session macro row. */
export interface RecorderMacro {
  readonly name: string;
  readonly path: string | null;
  readonly steps: number;
}

/** One normalised paused playback. */
export interface RecorderPlayback {
  readonly name: string;
  readonly nextStep: number;
  readonly total: number;
}

/** One normalised `cad_action_recorder` answer. */
export interface RecorderAnswer {
  readonly recording: RecorderRecording | null;
  readonly macros: readonly RecorderMacro[];
  readonly lastMacro: string | null;
  readonly playback: RecorderPlayback | null;
  /** The last recorder refusal, or `null` after a successful recorder command. */
  readonly lastError: string | null;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function finiteCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function pathOf(value: unknown): string | null {
  return typeof value === 'string' && value !== '' ? value : null;
}

function recordingOf(value: unknown): RecorderRecording | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const name = typeof fields.name === 'string' ? fields.name : '';
  const steps = finiteCount(fields.steps);
  if (name === '' || steps === null) return null;
  return { name, path: pathOf(fields.path), steps };
}

function macroOf(value: unknown): RecorderMacro | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const name = typeof fields.name === 'string' ? fields.name : '';
  const steps = finiteCount(fields.steps);
  if (name === '' || steps === null) return null;
  return { name, path: pathOf(fields.path), steps };
}

function playbackOf(value: unknown): RecorderPlayback | null {
  const fields = recordOf(value);
  if (!fields) return null;
  const name = typeof fields.name === 'string' ? fields.name : '';
  const nextStep = finiteCount(fields.nextStep);
  const total = finiteCount(fields.total);
  if (name === '' || nextStep === null || total === null || nextStep < 1) return null;
  return { name, nextStep, total };
}

/**
 * Normalise one `cad_action_recorder` answer.
 *
 * The host is the authority: nothing is inferred. A malformed row is dropped
 * (it can neither be addressed nor safely played) rather than shown with a
 * fabricated step count.
 */
export function parseActionRecorderAnswer(answer: unknown): RecorderAnswer {
  const source = recordOf(answer);
  const rawMacros: unknown[] = Array.isArray(source?.macros) ? source.macros : [];
  const macros: RecorderMacro[] = [];
  const seen = new Set<string>();
  for (const raw of rawMacros) {
    const macro = macroOf(raw);
    if (!macro || seen.has(macro.name)) continue;
    seen.add(macro.name);
    macros.push(macro);
  }
  const lastMacro = typeof source?.lastMacro === 'string' && source.lastMacro !== ''
    ? source.lastMacro
    : null;
  return {
    recording: source?.recording === null ? null : recordingOf(source?.recording),
    macros,
    lastMacro,
    playback: source?.playback === null ? null : playbackOf(source?.playback),
    lastError: typeof source?.lastError === 'string' && source.lastError !== ''
      ? source.lastError
      : null,
  };
}

/** Whether typed text names an action-recorder control command. */
export function isActionRecorderControl(text: string): boolean {
  const first = text.trim().split(/\s+/, 1)[0]?.toUpperCase() ?? '';
  return ['ACTRECORD', 'ACTSTOP', '-ACTSTOP', 'ACTUSERINPUT', 'ACTUSERMESSAGE',
    '-ACTUSERMESSAGE', 'ACTMANAGER'].includes(first);
}

/** `ACTRECORD` / `ACTRECORD <name-or-path>`; `null` for an empty non-bare start. */
export function recordStartCommand(target: string): string {
  const tail = target.trim();
  return tail === '' ? 'ACTRECORD' : `ACTRECORD ${tail}`;
}

/** `ACTSTOP` / `ACTSTOP <path.json>`; the bare form uses ACTRECORD's pending path. */
export function recordStopCommand(path: string): string {
  const tail = path.trim();
  return tail === '' ? 'ACTSTOP' : `ACTSTOP ${tail}`;
}

/** Whether a typed stop destination looks like the required `.json` file. */
export function isJsonDestination(path: string): boolean {
  return /\.json$/i.test(path.trim());
}

/** `ACTMANAGER PLAY <name>`. */
export function recordPlayCommand(name: string): string {
  return `ACTMANAGER PLAY ${name}`;
}

/** `ACTMANAGER DELETE <name>`. */
export function recordDeleteCommand(name: string): string {
  return `ACTMANAGER DELETE ${name}`;
}

/** `ACTMANAGER CONTINUE`. */
export const RECORD_CONTINUE_COMMAND = 'ACTMANAGER CONTINUE';

/** `ACTMANAGER LIST` (the panel may ask for the same read the host read serves). */
export const RECORD_LIST_COMMAND = 'ACTMANAGER LIST';
