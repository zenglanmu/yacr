// The Parameters Manager's wire contract, its pure normalisation and the typed
// `-PARAMETERS` command builders (ADR-0043 D9).
//
// The host owns the parameter table. The panel is a **view plus a command
// writer**: it reads `cad_param_list` (the `paramList` host read) and sends one
// `-PARAMETERS …` line per edit through `cad_command`, so every write is the one
// merged parametric transaction (one revision, one undo step) and every refusal
// is the host's own `param.*` reason.
//
// # What this build does and does not manage
//
// `PARAMETERS` is AutoCAD's Parameters Manager, which in AutoCAD also lists
// **dimensional/geometric constraint parameters** produced by a constraint
// solver. This build implements no such solver, so those rows do not exist and
// the panel says so in one explicit line; what it manages is the document's
// user-parameter table (create / value or expression / rename / delete).
//
// # Wire shape (frozen, `cad_param_list`)
//
// ```jsonc
// { "active": "Small" | null,
//   "rows": [{ "seq": 1, "name": "r", "kind": "length", "unit": "mm",
//              "value": 5.0, "status": "overridden" }],
//   "total": 1 }
// ```
//
// Nothing is fabricated: a row without a usable sequence or name is dropped (it
// cannot be shown or addressed), a non-finite value renders as `—`, and an
// unknown kind/status token is shown raw instead of being mapped to a claim.

/** The wire read: the document-level parameter list. */
export const PARAM_LIST_COMMAND = 'cad_param_list';
/** The one command funnel every write rides. */
export const PARAM_COMMAND = 'cad_command';
/** The chrome-local panel word. */
export const PARAMETERS_COMMAND = 'PARAMETERS';
/** The chrome-local idempotent close word. */
export const PARAMETERS_CLOSE_COMMAND = 'PARAMETERSCLOSE';
/** The typed command-line spelling the panel's writes use. */
export const DASH_PARAMETERS_COMMAND = '-PARAMETERS';

/** The three parameter kinds this build stores. */
export const PARAM_KINDS = Object.freeze(['length', 'number', 'angle'] as const);

/** One parameter kind token. */
export type ParamKindToken = typeof PARAM_KINDS[number];

/** One normalised row of the host's answer. */
export interface ParamRow {
  /** The base table sequence (identity across renames); always `>= 1`. */
  readonly seq: number;
  /** The exact stored name. */
  readonly name: string;
  /** `length` | `number` | `angle`, or the host's raw token when unknown. */
  readonly kind: string;
  /** The canonical/declared unit token (`mm`, `rad`), or `null`. */
  readonly unit: string | null;
  /** The effective value, or `null` when the host sent no finite number. */
  readonly value: number | null;
  /** `inherited` | `overridden` | `suppressed`, or the host's raw token. */
  readonly status: string;
}

/** One normalised `cad_param_list` answer. */
export interface ParamListAnswer {
  /** The active configuration, or `null` when the base table is in force. */
  readonly active: string | null;
  readonly rows: readonly ParamRow[];
  /** The host's own total. */
  readonly total: number;
}

/** Whether typed text is the bare `PARAMETERS` selector (`parameters` included). */
export function isParametersCommand(text: unknown): boolean {
  return normalize(text) === PARAMETERS_COMMAND;
}

/** Whether typed text is the bare `PARAMETERSCLOSE` selector. */
export function isParametersCloseCommand(text: unknown): boolean {
  return normalize(text) === PARAMETERS_CLOSE_COMMAND;
}

function normalize(text: unknown): string {
  return String(text ?? '').trim().toUpperCase();
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function recordOf(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/**
 * Normalise one `cad_param_list` answer.
 *
 * The host is the authority: nothing is added, inferred, re-ordered or case-
 * folded. A row is kept only with a positive integer sequence and a non-empty
 * name (anything else can neither be shown nor addressed), and a repeated
 * sequence is dropped after its first occurrence — one sequence is one row.
 */
export function parseParamListAnswer(answer: unknown): ParamListAnswer {
  const source = recordOf(answer);
  const rawRows: unknown[] = Array.isArray(source?.rows) ? source.rows : [];
  const rows: ParamRow[] = [];
  const seen = new Set<number>();
  for (const raw of rawRows) {
    const fields = recordOf(raw);
    if (!fields) continue;
    const seq = typeof fields.seq === 'number' && Number.isSafeInteger(fields.seq) && fields.seq >= 1
      ? fields.seq
      : null;
    const name = typeof fields.name === 'string' ? fields.name : '';
    if (seq === null || name === '' || seen.has(seq)) continue;
    seen.add(seq);
    const value = typeof fields.value === 'number' && Number.isFinite(fields.value)
      ? fields.value
      : null;
    rows.push({
      seq,
      name,
      kind: typeof fields.kind === 'string' && fields.kind !== '' ? fields.kind : 'unknown',
      unit: typeof fields.unit === 'string' && fields.unit !== '' ? fields.unit : null,
      value,
      status: typeof fields.status === 'string' && fields.status !== '' ? fields.status : 'unknown',
    });
  }
  const active = typeof source?.active === 'string' && source.active !== '' ? source.active : null;
  const reported = source?.total;
  const total = typeof reported === 'number' && Number.isSafeInteger(reported) && reported >= 0
    ? reported
    : rows.length;
  return { active, rows, total };
}

/** The catalog key naming one kind, or `null` for a token this build cannot name. */
export function paramKindKey(kind: string): string | null {
  return kind === 'length' ? 'parameters.kind.length'
    : kind === 'number' ? 'parameters.kind.number'
      : kind === 'angle' ? 'parameters.kind.angle'
        : null;
}

/** The catalog key naming one configuration status, or `null`. */
export function paramStatusKey(status: string): string | null {
  return status === 'inherited' ? 'parameters.status.inherited'
    : status === 'overridden' ? 'parameters.status.overridden'
      : status === 'suppressed' ? 'parameters.status.suppressed'
        : null;
}

/**
 * A parameter number without invented precision.
 *
 * The wire value is an `f64`; this trims the binary representation's trailing
 * zeros (so `6.0` reads `6`) and never rounds a significant digit away.
 */
export function formatParamNumber(value: number): string {
  if (!Number.isFinite(value)) return '\u2014';
  const text = String(value);
  return text.includes('e') || text.includes('E') ? text : text.replace(/\.0+$/, '');
}

/** The row's value with its unit (`6 mm`, `0.5 rad`), or `—` when unusable. */
export function formatParamValue(row: ParamRow): string {
  if (row.value === null) return '\u2014';
  const number = formatParamNumber(row.value);
  return row.unit === null ? number : `${number} ${row.unit}`;
}

/**
 * One name as a `-PARAMETERS` token: bare when it has no whitespace, quoted
 * otherwise.
 *
 * `null` means the stored name carries a double quote, which the typed grammar
 * has no escape for — the panel refuses to build a command rather than sending a
 * spelling that addresses a different (or no) row.
 */
export function paramTargetToken(name: string): string | null {
  if (name === '' || name.includes('"') || /[\r\n]/.test(name)) return null;
  return /\s/.test(name) ? `"${name}"` : name;
}

/** One `-PARAMETERS NEW <name> <kind> <expr>` line, or `null` when unbuildable. */
export function createParamCommand(name: string, kind: string, expr: string): string | null {
  const target = paramTargetToken(name);
  const expression = expr.trim();
  if (target === null || expression === '' || !isKindToken(kind)) return null;
  return `${DASH_PARAMETERS_COMMAND} NEW ${target} ${kind} ${expression}`;
}

/** One `-PARAMETERS SET <name> <expr>` line, or `null`. */
export function setParamValueCommand(name: string, expr: string): string | null {
  const target = paramTargetToken(name);
  const expression = expr.trim();
  return target === null || expression === '' ? null : `${DASH_PARAMETERS_COMMAND} SET ${target} ${expression}`;
}

/** One `-PARAMETERS RENAME <old> <new>` line, or `null`. */
export function renameParamCommand(name: string, next: string): string | null {
  const target = paramTargetToken(name);
  const renamed = paramTargetToken(next);
  return target === null || renamed === null ? null : `${DASH_PARAMETERS_COMMAND} RENAME ${target} ${renamed}`;
}

/** One `-PARAMETERS DELETE <name>` line, or `null`. */
export function deleteParamCommand(name: string): string | null {
  const target = paramTargetToken(name);
  return target === null ? null : `${DASH_PARAMETERS_COMMAND} DELETE ${target}`;
}

/** Whether a token is one of the three kinds this build stores. */
export function isKindToken(kind: unknown): kind is ParamKindToken {
  return typeof kind === 'string' && (PARAM_KINDS as readonly string[]).includes(kind);
}
