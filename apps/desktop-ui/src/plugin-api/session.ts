// The one plugin entry point: assemble the client and hand it back.
//
// This module only assembles: it calls the bridge's `hostInfo()` **once**, passes
// that same memoized promise to `createCadApi` (which still owns the
// `apiVersion` compatibility check and the capability gate), and returns the
// client together with the snapshot it was built from.
//
// No React, no module loading, no filesystem access: loading a plugin's ES
// module and calling `activate(ctx)` is later host work, and keeping it out is
// what makes this module a pure assembly step.

import { createCadApi } from '../../../../packages/cad-plugin-api/src/index.ts';
import type {
  CadApi,
  CadApiHostInfo,
  CadHostBridge,
  CadApiRequest,
} from '../../../../packages/cad-plugin-api/src/index.ts';
import { createCadHostBridge } from './bridge.ts';
import type { CadBridgeDeps } from './bridge.ts';

/** What assembling the client needs; the same deps the bridge takes. */
export interface PluginApiDeps extends CadBridgeDeps {}

/** The assembled client plus the host snapshot it agrees with. */
export interface PluginApiSession {
  readonly api: CadApi;
  readonly host: CadApiHostInfo;
}

/**
 * Build the plugin client for this carrier.
 *
 * Exactly one `cad_api_status` call happens here: `hostInfo()` is invoked once
 * and the resulting promise answers both `createCadApi`'s compatibility check and
 * the caller's `host` field. A host refusal or an incompatible `apiVersion`
 * rejects this function; there is no fallback client.
 */
export async function openPluginApi(deps: PluginApiDeps): Promise<PluginApiSession> {
  const bridge = createCadHostBridge(deps);
  const host = bridge.hostInfo();
  const memoized: CadHostBridge = {
    hostInfo: () => host,
    request: <Result>(request: CadApiRequest): Promise<Result> => bridge.request<Result>(request),
    mutate: <Result>(request: CadApiRequest): Promise<Result> => bridge.mutate<Result>(request),
  };
  const api = await createCadApi(memoized);
  return { api, host: await host };
}
