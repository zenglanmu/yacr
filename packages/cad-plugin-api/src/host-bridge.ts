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
 * What a carrier implements.
 *
 * `request` may reject with a `CadApiError` or with a plain `{ code, message,
 * details }` object (the shaped JSON a Rust host sends); both are mapped to
 * `CadApiError` with the same code. A `null`/`undefined` answer is refused.
 */
export interface CadHostBridge {
  hostInfo(): Promise<CadApiHostInfo>;
  request<Result>(request: CadApiRequest): Promise<Result>;
}
