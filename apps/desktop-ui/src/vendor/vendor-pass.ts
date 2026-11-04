// The WebView half of the vendor pass: fetch records → decode in the plugin →
// post the result back.
//
// The plugin is a **separately built ES module** under `public/plugins/`
// (`plugins/tch/index.ts`). It is loaded with a dynamic `import()` from the
// document root, so the same file works in the Tauri WebView and in the browser
// bundle. The app never imports plugin source: the only contract is
// `decode(ArrayBuffer) -> ArrayBuffer`.
//
// Failure closure: a missing plugin, a malformed frame or an ABI mismatch is
// turned into a real **unsupported** result frame and posted to the host, so the
// pass settles with a diagnostic instead of hanging in `pending`.

import { invoke } from '../host/invoke.ts';

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

const VENDOR_ABI_VERSION = 4;
const RESULT_MAGIC = 0x59414e52;
const RESULT_STATUS_UNSUPPORTED = 1;

/** The plugin URL, resolved against the document root (Tauri and browser alike). */
function pluginUrl(): string {
  return new URL('plugins/tch.js', document.baseURI).href;
}

let plugin: Promise<VendorPluginModule> | null = null;

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

/**
 * Load and validate the plugin module once per page.
 *
 * A `<script type="module">` is used instead of a dynamic `import()`: Vite's dev
 * transform rewrites `import(url)` and the dev server refuses public-dir files,
 * which is why the canvas bootstrap uses the same injected-script pattern.
 */
export function loadVendorPlugin(): Promise<VendorPluginModule> {
  if (plugin !== null) return plugin;
  plugin = new Promise<VendorPluginModule>((resolve, reject) => {
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
  return plugin;
}

/** Test seam: forget the cached module (never called by the app). */
export function resetVendorPluginForTests(): void {
  plugin = null;
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

/** An ABI v4 result frame carrying one unsupported diagnostic. */
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
  // ABI v4: zero entity groups. An unsupported result must never look like a
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
export async function runVendorPass(frame: VendorPendingFrame): Promise<VendorAck> {
  let result: ArrayBuffer;
  try {
    const records = asArrayBuffer(
      await invoke<unknown>('cad_vendor_records', { token: frame.token }),
    );
    const module = await loadVendorPlugin();
    if (module.abiVersion !== VENDOR_ABI_VERSION) {
      throw new Error(`vendor.unsupported_abi: ${module.abiVersion}`);
    }
    result = module.decode(records);
  } catch (error) {
    result = unsupportedResult(frame.token, frame.generation, `vendor.decode_failed: ${String(error)}`);
  }
  return invoke<VendorAck>('cad_vendor_apply', {
    token: frame.token,
    generation: frame.generation,
    payload: encodeBase64(result),
  });
}
