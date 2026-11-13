// The one error type a plugin ever sees from this package.
//
// `code` is the machine-readable contract and is stable across versions; the
// message is for humans and may change. Callers must branch on `code`, never on
// the message text.
//
// Stable codes:
//   `api.pending: <namespace>.<operation>`
//   `unsupported-runtime: cad_api.<namespace>.<operation>`   (explicit `unsupported` row)
//   `api.descriptor_missing: <namespace>.<operation>`         (row absent from the host table)
//   `api.descriptor_invalid: <detail>`                        (host descriptor contradicts itself)
//   `api.revision-conflict`               (details.actualRevision when the host knows it)
//   `api.transaction.nesting_order`
//   `api.transaction.nesting_limit`
//   `api.transaction.closed`
//   `api.invalid_argument: <field>`       (includes `edits.duplicate` and `bridge.answer`)
//   `api.manifest.<reason>`               (e.g. `api.manifest.unknown_key: extra`)
//   `api.host-outdated: <apiVersion>`

/** Extra context for a `CadApiError`; every field is optional. */
export interface CadApiErrorInit {
  /** `<namespace>.<operation>` the failure belongs to, when it belongs to one. */
  readonly operation?: string;
  /** Structured host-supplied context (e.g. `{ actualRevision }`). */
  readonly details?: Readonly<Record<string, unknown>>;
  /** Underlying cause, preserved for debugging. */
  readonly cause?: unknown;
}

/** The single error type raised by this package (and expected from a bridge). */
export class CadApiError extends Error {
  /** Stable machine-readable code; branch on this, not on the message. */
  readonly code: string;
  /** `<namespace>.<operation>` when the failure is tied to one operation. */
  readonly operation?: string;
  /** Host-supplied structured context, never invented locally. */
  readonly details?: Readonly<Record<string, unknown>>;

  constructor(code: string, message?: string, init?: CadApiErrorInit) {
    super(message ?? code);
    this.name = 'CadApiError';
    this.code = code;
    this.operation = init?.operation;
    this.details = init?.details;
    if (init?.cause !== undefined) this.cause = init.cause;
  }
}

/** Narrowing helper; safe on values that crossed the bridge as plain objects. */
export function isCadApiError(value: unknown): value is CadApiError {
  return value instanceof CadApiError;
}

/** The stable code of a `CadApiError`, or `undefined` for anything else. */
export function errorCodeOf(value: unknown): string | undefined {
  return value instanceof CadApiError ? value.code : undefined;
}

/**
 * `api.invalid_argument: <field>`.
 *
 * Used for a locally-rejected argument *and* for a host answer that cannot be
 * understood: an unintelligible or missing answer is refused, never coerced into
 * a placeholder value.
 */
export function invalidArgument(field: string): CadApiError {
  return new CadApiError(`api.invalid_argument: ${field}`, `invalid argument: ${field}`);
}
