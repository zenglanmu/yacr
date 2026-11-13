// The host boundary: everything a carrier must implement, and nothing else.
//
// This is the only place the package describes transport. There is no fetch, no
// Tauri import, no wasm binding here — a carrier supplies a `CadHostBridge` and
// the package never learns which carrier it is talking to beyond the reported
// snapshot (`carrier`/`documentOwner`), which is informational only.
//
// The bridge is untrusted input: `createCadApi` validates the host info shape,
// and every answer that cannot be understood is refused as
// `api.invalid_argument: bridge.answer` rather than defaulted.

import type { CadApiNamespace, CadApiOperation } from './abi.ts';

/** The host's self-description. Read once, at `createCadApi` time. */
export interface CadApiHostInfo {
  /** Must equal `API_VERSION`; anything else is `api.host-outdated: <version>`. */
  readonly apiVersion: number;
  /** Which carrier answers. Informational: routing never depends on it here. */
  readonly carrier: 'native-below' | 'dom-canvas' | 'unknown';
  /** Which side owns the `cad-db` document. Informational, for diagnostics. */
  readonly documentOwner: 'native' | 'wasm' | 'none';
  /** The host's own capability rows; this is what calls are gated against. */
  readonly operations: readonly CadApiOperation[];
}

/** One capability-gated wire request. */
export interface CadApiRequest {
  readonly namespace: CadApiNamespace;
  readonly operation: string;
  readonly parameters: Readonly<Record<string, unknown>>;
}

/**
 * What a carrier implements: two lanes, deliberately separate.
 *
 * `request` is the **read** lane — one synchronous answer reflected from the
 * host's snapshot. `mutate` is the **write** lane — the host queues the request
 * and the carrier resolves only once the host reported the outcome (the Tauri /
 * wasm carriers do that with `cad_api_submit` + `cad_api_poll`; the correlation
 * id and the polling never leave the carrier).
 *
 * Keeping them apart is what makes a commit honest: it cannot resolve before the
 * host said what happened, and a read can never be mistaken for a write. Both may
 * reject with a `CadApiError` or with a plain `{ code, message, details }` object
 * (the shaped JSON a Rust host sends); both are mapped to `CadApiError` with the
 * same code. A `null`/`undefined` answer is refused.
 */
export interface CadHostBridge {
  hostInfo(): Promise<CadApiHostInfo>;
  /** Read lane: gate, ask the host's reflected snapshot, return its answer. */
  request<Result>(request: CadApiRequest): Promise<Result>;
  /** Write lane: submit the mutation and resolve the host's reported outcome. */
  mutate<Result>(request: CadApiRequest): Promise<Result>;
}
