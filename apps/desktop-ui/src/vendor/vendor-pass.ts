// The WebView half of the vendor pass: fetch records → decode in the plugin →
// post the result back.
//
// The **active decoder** comes from the shell's plugin registry
// (`cad_plugin_status().activeDecoder`). The bundled decoder is a separately
// built ES module under `public/plugins/` (`plugins/tch/index.ts`): it is loaded
// with an injected module script from the document root, so the same file works
// in the Tauri WebView and in the browser bundle. An installed (non-`tch`)
// decoder is read from the shell as base64 and imported from a Blob URL. The app
// never imports plugin source: the only contract is
// `decode(ArrayBuffer) -> ArrayBuffer` on `__YACR_VENDOR_PLUGIN__`.
//
// Failure closure: a disabled/absent decoder, a missing plugin, a malformed
// frame or an ABI mismatch is turned into a real **unsupported** result frame and
// posted to the host, so the pass settles with a diagnostic instead of hanging in
// `pending`.

import { invoke } from '../host/invoke.ts';
import { bytesFromBase64, BUILTIN_PLUGIN_ID, readPluginStatus } from '../plugins/plugin-manager.ts';
import type { PluginImporter, PluginInvoke, PluginStatusFrame } from '../plugins/types.ts';

/** The `vendor-pending` frame the host emits after a document opens. */
export interface VendorPendingFrame {
  readonly token: number;
  readonly generation: number;
  readonly entities: number;
}

/** The host's acknowledgement for one settled pass. */
export interface VendorAck {
  readonly accepted: boolean;
  readonly applied: number;
  readonly diagnostics: readonly string[];
}

/** The plugin module shape (the only exports the host relies on). */
export interface VendorPluginModule {
  readonly abiVersion: number;
  decode(records: ArrayBuffer): ArrayBuffer;
}

const VENDOR_ABI_VERSION = 1;
const RESULT_MAGIC = 0x59414e52;
const RESULT_STATUS_UNSUPPORTED = 1;

/** The plugin URL, resolved against the document root (Tauri and browser alike). */
function pluginUrl(): string {
  return new URL('plugins/tch.js', document.baseURI).href;
}

/** The injection seams a test can replace (production uses the real ones). */
export interface VendorPluginLoaderDeps {
  /** The shell `invoke`; defaults to the transport seam (`host/invoke.ts`). */
  readonly invoke?: PluginInvoke;
  /** The external-entry importer; defaults to a dynamic import of the Blob URL. */
  readonly importModule?: PluginImporter;
}

let plugin: Promise<VendorPluginModule> | null = null;
let pluginStatus: Promise<PluginStatusFrame> | null = null;

/** The global the plugin module registers itself on. */
interface VendorGlobal {
  __YACR_VENDOR_PLUGIN__?: VendorPluginModule;
}

function validated(candidate: Partial<VendorPluginModule> | undefined): VendorPluginModule {
  if (!candidate || typeof candidate.decode !== 'function' || typeof candidate.abiVersion !== 'number') {
    throw new Error('vendor.plugin_contract');
  }
  return candidate as VendorPluginModule;
}

/** A plain JSON object, or `null` for anything else (arrays included). */
function plainRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** The active-decoder snapshot, read once per page (the same cache as the plugin). */
function pluginStatusOf(deps: VendorPluginLoaderDeps): Promise<PluginStatusFrame> {
  if (pluginStatus === null) {
    pluginStatus = readPluginStatus(deps.invoke ?? invoke);
  }
  return pluginStatus;
}

/** The bundled decoder: a `<script type="module">` from the document root. */
function injectBundledPlugin(): Promise<VendorPluginModule> {
  return new Promise<VendorPluginModule>((resolve, reject) => {
    const existing = (globalThis as VendorGlobal).__YACR_VENDOR_PLUGIN__;
    if (existing) {
      try {
        resolve(validated(existing));
      } catch (error) {
        reject(error);
      }
      return;
    }
    const script = document.createElement('script');
    script.type = 'module';
    script.src = pluginUrl();
    script.onload = () => {
      try {
        resolve(validated((globalThis as VendorGlobal).__YACR_VENDOR_PLUGIN__));
      } catch (error) {
        reject(error);
      }
    };
    script.onerror = () => reject(new Error('vendor.plugin_load_failed'));
    document.head.append(script);
  });
}

/** The production importer for an installed (external) decoder entry. */
const defaultImporter: PluginImporter = url => import(/* @vite-ignore */ url);

/**
 * An installed (non-`tch`) decoder entry: read the shell's stored bytes and
 * import them as a Blob URL module.
 *
 * The plugin still registers on `__YACR_VENDOR_PLUGIN__`, so the validation is
 * the same as the bundled one. The URL is revoked whether the import succeeds or
 * fails; nothing is cached here beyond the module promise.
 */
async function loadExternalPlugin(
  id: string,
  deps: VendorPluginLoaderDeps,
): Promise<VendorPluginModule> {
  const call = deps.invoke ?? invoke;
  const answer = await call('cad_plugin_read_entry', { id });
  const record = plainRecord(answer);
  const payload = record?.payload;
  if (typeof payload !== 'string') throw new Error('vendor.plugin_entry_invalid');
  const source = bytesFromBase64(payload);
  // `slice()` copies into an `ArrayBuffer`-backed view, which is what `BlobPart`
  // accepts (a raw `Uint8Array<ArrayBufferLike>` may be SharedArrayBuffer-backed).
  const url = URL.createObjectURL(new Blob([source.slice()], { type: 'text/javascript' }));
  try {
    await (deps.importModule ?? defaultImporter)(url);
  } finally {
    URL.revokeObjectURL(url);
  }
  return validated((globalThis as VendorGlobal).__YACR_VENDOR_PLUGIN__);
}

/**
 * Load and validate the **active** decoder module once per page.
 *
 * `cad_plugin_status().activeDecoder` decides what runs:
 *
 * * `null` - no decoder configured; reject with the stable
 *   `vendor.decoder_disabled` (the pass then settles as a real unsupported
 *   result instead of hanging in `pending`);
 * * `tch` - the bundled `plugins/tch.js`, injected as a module script (Vite's
 *   dev transform rewrites a dynamic `import(url)` and refuses public-dir
 *   files, which is why the bootstrap uses the same injected-script pattern);
 * * any other id - the shell's installed entry, read as base64, imported from a
 *   Blob URL, then validated on the same global.
 */
export function loadVendorPlugin(deps: VendorPluginLoaderDeps = {}): Promise<VendorPluginModule> {
  if (plugin !== null) return plugin;
  plugin = (async () => {
    const status = await pluginStatusOf(deps);
    // An unsupported carrier has no registry to persist "no decoder", but the
    // bundled `tch` module is still shipped and always-on there (Linux
    // `dom-canvas`, Web). Only a supported shell can deliberately disable it.
    const active = status.supported ? status.activeDecoder : (status.activeDecoder ?? BUILTIN_PLUGIN_ID);
    if (active === null) throw new Error('vendor.decoder_disabled');
    if (active === BUILTIN_PLUGIN_ID) return injectBundledPlugin();
    return loadExternalPlugin(active, deps);
  })();
  return plugin;
}

/** Test seam: forget the cached module (never called by the app). */
export function resetVendorPluginForTests(): void {
  plugin = null;
}

/** Test seam: forget the cached module **and** the cached status snapshot. */
export function resetVendorPluginLoaderForTests(): void {
  plugin = null;
  pluginStatus = null;
}

/** Accept the ArrayBuffer the Tauri raw response produces, or a byte view. */
function asArrayBuffer(value: unknown): ArrayBuffer {
  if (value instanceof ArrayBuffer) return value;
  if (ArrayBuffer.isView(value)) {
    const view = value as ArrayBufferView;
    return view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength) as ArrayBuffer;
  }
  if (Array.isArray(value)) return Uint8Array.from(value as number[]).buffer;
  throw new Error('vendor.records_not_binary');
}

/** An ABI v1 result frame carrying one unsupported diagnostic. */
export function unsupportedResult(token: number, generation: number, message: string): ArrayBuffer {
  const diagnostics = [message];
  const encoder = new TextEncoder();
  const encoded = diagnostics.map(item => encoder.encode(item));
  let length = 40;
  for (const bytes of encoded) length += 2 + bytes.byteLength;
  const buffer = new ArrayBuffer(length);
  const view = new DataView(buffer);
  view.setUint32(0, RESULT_MAGIC, true);
  view.setUint32(4, VENDOR_ABI_VERSION, true);
  view.setBigUint64(8, BigInt(token), true);
  view.setBigUint64(16, BigInt(generation), true);
  view.setUint32(24, RESULT_STATUS_UNSUPPORTED, true);
  view.setUint32(28, 0, true);
  view.setUint32(32, encoded.length, true);
  let offset = 36;
  for (const bytes of encoded) {
    view.setUint16(offset, bytes.byteLength, true);
    offset += 2;
    new Uint8Array(buffer, offset, bytes.byteLength).set(bytes);
    offset += bytes.byteLength;
  }
  // ABI v1: zero entity groups. An unsupported result must never look like a
  // truncated frame or claim primitives it does not carry.
  view.setUint32(offset, 0, true);
  return buffer;
}

/** Base64 of a byte buffer (the host accepts base64 on every carrier). */
export function encodeBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/**
 * Run one pass to completion and return the host's acknowledgement.
 *
 * A stale token (the user opened another file while decoding) is rejected by the
 * host with `vendor.stale_token`; the caller may ignore that specific rejection.
 */
export async function runVendorPass(
  frame: VendorPendingFrame,
  deps: VendorPluginLoaderDeps = {},
): Promise<VendorAck> {
  const call = deps.invoke ?? invoke;
  let result: ArrayBuffer;
  try {
    const module = await loadVendorPlugin(deps);
    if (module.abiVersion !== VENDOR_ABI_VERSION) {
      throw new Error(`vendor.unsupported_abi: ${module.abiVersion}`);
    }
    const records = asArrayBuffer(
      await call('cad_vendor_records', { token: frame.token }),
    );
    result = module.decode(records);
  } catch (error) {
    const message = String(error);
    // A disabled decoder is a deliberate configuration, not a decode failure:
    // report the stable reason verbatim so the host can surface it as such.
    const reason = message.includes('vendor.decoder_disabled')
      ? 'vendor.decoder_disabled'
      : `vendor.decode_failed: ${message}`;
    result = unsupportedResult(frame.token, frame.generation, reason);
  }
  return (await call('cad_vendor_apply', {
    token: frame.token,
    generation: frame.generation,
    payload: encodeBase64(result),
  })) as VendorAck;
}
