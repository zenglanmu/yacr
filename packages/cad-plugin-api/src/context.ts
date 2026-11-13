// The plugin-facing entry: what a plugin receives, how it is declared, and how
// the client is assembled from a host bridge.
//
// Lifecycle: a plugin calls `definePlugin` (identity + shape validation), the host
// loads the ES module named by the manifest `entry`, and the host calls
// `activate(ctx)` once and `deactivate(ctx)` once. This package does not load
// modules, resolve paths, or manage timers — that is host work, and keeping it
// out is what makes the client carrier-agnostic.
//
// `parsePluginManifest` is pure: it takes the manifest **text**, never a path, so
// there is no filesystem access and no ambient "current directory" in the
// contract. Unknown top-level keys are rejected: a manifest this version does not
// understand is not silently half-applied.

import { API_NAMESPACES, API_VERSION } from './abi.ts';
import type { CadApiNamespace, CadApiOperation, CadApiOperationStatus } from './abi.ts';
import { createApplication } from './application-services.ts';
import type { Application } from './application-services.ts';
import { createColorLibrary } from './colors.ts';
import type { ColorLibrary } from './colors.ts';
import { createEditor } from './editor-input.ts';
import type { Editor } from './editor-input.ts';
import { CadApiError, invalidArgument } from './errors.ts';
import { createGeometryLibrary } from './geometry.ts';
import type { GeometryLibrary } from './geometry.ts';
import type { CadApiHostInfo, CadHostBridge } from './host-bridge.ts';
import { createWire, errorFromBridgeFailure, isRecord } from './internal/capability.ts';
import { createRuntimeNamespace } from './runtime.ts';
import type { CommandRegistration, RuntimeNamespace } from './runtime.ts';

/** Everything a plugin may call. */
export interface CadApi {
  readonly application: Application;
  readonly editor: Editor;
  readonly geometry: GeometryLibrary;
  readonly colors: ColorLibrary;
  /** Command registration; client-local, reported to the host's hook. */
  readonly runtime: RuntimeNamespace;
}

/**
 * What the host may hand `createCadApi` so a plugin can register commands.
 *
 * `onCommandRegister` is the funnel's installation point: without it
 * `runtime.registerCommand` refuses, because the carrier has nowhere to dispatch
 * the command. `onCommandUnregister` is its inverse.
 */
export interface CadApiOptions {
  readonly onCommandRegister?: (registration: CommandRegistration) => void;
  readonly onCommandUnregister?: (name: string) => void;
  /**
   * Where `editor.writeMessage()` puts its text.
   *
   * `writeMessage` is client-local (it has no descriptor row and no wire call), so
   * a host that does not supply a sink gets a refusal from that method instead of
   * a silently dropped message.
   */
  readonly onMessage?: (text: string) => void;
}

/** What the host passes to `activate`/`deactivate`. */
export interface PluginContext {
  readonly api: CadApi;
  readonly host: CadApiHostInfo;
  readonly pluginId: string;
}

/** A plugin module's default export shape. */
export interface CadPlugin {
  readonly id: string;
  readonly apiVersion: number;
  activate?(ctx: PluginContext): void | Promise<void>;
  deactivate?(ctx: PluginContext): void | Promise<void>;
}

/** The on-disk manifest a host reads before importing the plugin module. */
export interface PluginManifest {
  readonly manifestVersion: 1;
  readonly id: string;
  readonly name: string;
  readonly version: string;
  readonly apiVersion: number;
  readonly entry: string;
  readonly commands?: readonly string[];
}

const PLUGIN_ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/;
const COMMAND_PATTERN = /^[A-Z][A-Z0-9_]{0,31}$/;
const MANIFEST_KEYS: readonly string[] = [
  'manifestVersion',
  'id',
  'name',
  'version',
  'apiVersion',
  'entry',
  'commands',
];
const STATUSES: readonly CadApiOperationStatus[] = ['supported', 'pending', 'unsupported'];

// Manifest failures belong to the package, not to a host operation: the parser is
// a local feature with no descriptor row, so these errors carry no `operation`.
function manifestError(reason: string): CadApiError {
  return new CadApiError(`api.manifest.${reason}`, `plugin manifest rejected: ${reason}`);
}

function invalidManifest(field: string): CadApiError {
  return new CadApiError('api.manifest.invalid', `plugin rejected: ${field}`);
}

/**
 * Declare a plugin. Identity + shape validation, no side effects.
 *
 * A mismatch with `API_VERSION` is refused here rather than at activation: a
 * plugin built against another contract generation should never be loaded.
 */
export function definePlugin(plugin: CadPlugin): CadPlugin {
  if (typeof plugin !== 'object' || plugin === null) throw invalidManifest('plugin');
  if (typeof plugin.id !== 'string' || !PLUGIN_ID_PATTERN.test(plugin.id)) {
    throw invalidManifest('id');
  }
  if (plugin.apiVersion !== API_VERSION) throw invalidManifest('apiVersion');
  if (plugin.activate !== undefined && typeof plugin.activate !== 'function') {
    throw invalidManifest('activate');
  }
  if (plugin.deactivate !== undefined && typeof plugin.deactivate !== 'function') {
    throw invalidManifest('deactivate');
  }
  return plugin;
}

/**
 * Validate one manifest string and return the frozen manifest.
 *
 * Pure: no filesystem, no `import`, no environment. Stable refusal reasons are
 * `api.manifest.<reason>`, with the offending key appended for unknown keys
 * (`api.manifest.unknown_key: <key>`).
 */
export function parsePluginManifest(text: string): PluginManifest {
  if (typeof text !== 'string') throw invalidArgument('text');

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (cause) {
    throw new CadApiError('api.manifest.invalid_json', 'plugin manifest is not valid JSON', {
      cause,
    });
  }
  if (!isRecord(parsed)) throw manifestError('not_object');
  const record = parsed;

  for (const key of Object.keys(record)) {
    if (!MANIFEST_KEYS.includes(key)) throw manifestError(`unknown_key: ${key}`);
  }

  if (record['manifestVersion'] !== 1) throw manifestError('manifest_version');

  const id = record['id'];
  if (typeof id !== 'string' || !PLUGIN_ID_PATTERN.test(id)) throw manifestError('id');

  const name = record['name'];
  if (typeof name !== 'string' || name.trim().length === 0) throw manifestError('name');

  const version = record['version'];
  if (typeof version !== 'string' || version.trim().length === 0) throw manifestError('version');

  if (record['apiVersion'] !== API_VERSION) throw manifestError('api_version');

  const entry = record['entry'];
  if (typeof entry !== 'string') throw manifestError('entry');
  if (!entry.endsWith('.js') && !entry.endsWith('.mjs')) throw manifestError('entry');
  if (entry.includes('..') || entry.includes('\\')) throw manifestError('entry');
  if (entry.startsWith('/') || /^[A-Za-z]:/.test(entry)) throw manifestError('entry');

  const rawCommands = record['commands'];
  let commands: readonly string[] | undefined;
  if (rawCommands !== undefined) {
    if (!Array.isArray(rawCommands)) throw manifestError('commands');
    const seen = new Set<string>();
    const collected: string[] = [];
    for (const command of rawCommands) {
      if (typeof command !== 'string' || !COMMAND_PATTERN.test(command)) {
        throw manifestError('commands');
      }
      if (seen.has(command)) throw manifestError('commands');
      seen.add(command);
      collected.push(command);
    }
    commands = collected;
  }

  const manifest: PluginManifest = {
    manifestVersion: 1,
    id,
    name,
    version,
    apiVersion: API_VERSION,
    entry,
    ...(commands === undefined ? {} : { commands }),
  };
  return Object.freeze(manifest);
}

function hostOutdated(apiVersion: number): CadApiError {
  return new CadApiError(
    `api.host-outdated: ${apiVersion}`,
    `the host serves cad api ${apiVersion}, this package requires ${API_VERSION}`,
  );
}

function requireHostRecord(value: unknown, field: string): Readonly<Record<string, unknown>> {
  if (!isRecord(value)) throw invalidArgument(field);
  return value;
}

/** Narrowing guard for the contract's namespace list (no assertion at the call site). */
function isCadApiNamespace(value: string): value is CadApiNamespace {
  return API_NAMESPACES.some((namespace) => namespace === value);
}

/** Narrowing guard for the contract's status list. */
function isOperationStatus(value: string): value is CadApiOperationStatus {
  return STATUSES.some((status) => status === value);
}

function decodeOperation(value: unknown, index: number): CadApiOperation {
  const record = requireHostRecord(value, `hostInfo.operations[${index}]`);
  const namespace = record['namespace'];
  if (typeof namespace !== 'string' || !isCadApiNamespace(namespace)) {
    throw invalidArgument(`hostInfo.operations[${index}].namespace`);
  }
  const operation = record['operation'];
  if (typeof operation !== 'string' || operation.length === 0) {
    throw invalidArgument(`hostInfo.operations[${index}].operation`);
  }
  const status = record['status'];
  if (typeof status !== 'string' || !isOperationStatus(status)) {
    throw invalidArgument(`hostInfo.operations[${index}].status`);
  }
  const reason = record['reason'];
  if (reason !== undefined && typeof reason !== 'string') {
    throw invalidArgument(`hostInfo.operations[${index}].reason`);
  }
  const row: CadApiOperation = {
    namespace,
    operation,
    status,
    ...(reason === undefined ? {} : { reason }),
  };
  return Object.freeze(row);
}

function decodeHostInfo(record: Readonly<Record<string, unknown>>): CadApiHostInfo {
  const carrier = record['carrier'];
  if (carrier !== 'native-below' && carrier !== 'dom-canvas' && carrier !== 'unknown') {
    throw invalidArgument('hostInfo.carrier');
  }
  const documentOwner = record['documentOwner'];
  if (documentOwner !== 'native' && documentOwner !== 'wasm' && documentOwner !== 'none') {
    throw invalidArgument('hostInfo.documentOwner');
  }
  const operations = record['operations'];
  if (!Array.isArray(operations)) throw invalidArgument('hostInfo.operations');
  const rows = operations.map((row, index) => decodeOperation(row, index));
  return Object.freeze({
    apiVersion: API_VERSION,
    carrier,
    documentOwner,
    operations: Object.freeze(rows),
  });
}

/**
 * The document namespaces whose `supported` rows require a document owner.
 *
 * A descriptor that serves document operations while reporting
 * `documentOwner: 'none'` contradicts itself: there is a database behind those
 * calls, so it belongs to native or wasm. That is a host bug, refused here rather
 * than surfacing as a confusing failure on the first document call.
 */
function requiresDocumentOwner(namespace: CadApiNamespace): boolean {
  return namespace === 'ApplicationServices' || namespace === 'DatabaseServices';
}

function refuseContradiction(host: CadApiHostInfo): void {
  if (host.documentOwner !== 'none') return;
  const claimsDocuments = host.operations.some(
    (row) => row.status === 'supported' && requiresDocumentOwner(row.namespace),
  );
  if (!claimsDocuments) return;
  throw new CadApiError(
    'api.descriptor_invalid: documentOwner',
    'the descriptor serves document operations but reports documentOwner "none"',
  );
}

/**
 * Assemble the API for one host.
 *
 * Fails closed: an outdated host, a malformed or self-contradicting snapshot, or
 * a bridge that is not a bridge is refused before a plugin can call anything.
 *
 * Geometry, colors and the manifest parser are package features, not host
 * capabilities: they never cross the wire, need no descriptor row, and are always
 * available. Every host operation is gated against the host's own table and, in
 * this build, refuses by name (all 20 rows are `pending`).
 */
/**
 * Validate the host hooks. Absent hooks are fine (they mean "no funnel").
 */
function readOptions(options: CadApiOptions | undefined): CadApiOptions {
  if (options === undefined) return {};
  if (typeof options !== 'object' || options === null || Array.isArray(options)) {
    throw invalidArgument('options');
  }
  const onCommandRegister = options.onCommandRegister;
  if (onCommandRegister !== undefined && typeof onCommandRegister !== 'function') {
    throw invalidArgument('options.onCommandRegister');
  }
  const onCommandUnregister = options.onCommandUnregister;
  if (onCommandUnregister !== undefined && typeof onCommandUnregister !== 'function') {
    throw invalidArgument('options.onCommandUnregister');
  }
  const onMessage = options.onMessage;
  if (onMessage !== undefined && typeof onMessage !== 'function') {
    throw invalidArgument('options.onMessage');
  }
  return { onCommandRegister, onCommandUnregister, onMessage };
}

export async function createCadApi(
  bridge: CadHostBridge,
  options?: CadApiOptions,
): Promise<CadApi> {
  if (typeof bridge !== 'object' || bridge === null) throw invalidArgument('bridge');
  if (typeof bridge.hostInfo !== 'function') throw invalidArgument('bridge.hostInfo');
  if (typeof bridge.request !== 'function') throw invalidArgument('bridge.request');

  let raw: unknown;
  try {
    raw = await bridge.hostInfo();
  } catch (failure) {
    throw errorFromBridgeFailure(failure);
  }
  const record = requireHostRecord(raw, 'hostInfo');
  const apiVersion = record['apiVersion'];
  if (typeof apiVersion !== 'number' || !Number.isSafeInteger(apiVersion)) {
    throw invalidArgument('hostInfo.apiVersion');
  }
  if (apiVersion !== API_VERSION) throw hostOutdated(apiVersion);

  const host = decodeHostInfo(record);
  refuseContradiction(host);
  const wire = createWire(host, bridge);
  const hooks = readOptions(options);
  const editorOptions = { onMessage: hooks.onMessage };
  return {
    application: createApplication(wire, editorOptions),
    editor: createEditor(wire, editorOptions),
    geometry: createGeometryLibrary(),
    colors: createColorLibrary(),
    runtime: createRuntimeNamespace(hooks),
  };
}
