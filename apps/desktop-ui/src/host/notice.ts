// One-shot host notices, and the known status tokens they translate.
//
// The renderer lane reports a failed accelerated copy by putting a stable token
// at the start of the ordinary `cad_status.error` string:
//
//   gpu.pulled.insufficient-vram: required=2699194780 available=...
//
// That string can arrive on the **first** `status-changed`, after a gap
// recovery, and again on every snapshot while the fallback lasts. Turning each
// snapshot into a modal would make the warning impossible to dismiss, so this
// module owns a latch:
//
// * the token's first appearance (`false -> true`) opens the notice once;
// * the latch stays set while the token persists, so a dismissal is respected;
// * the token disappearing clears the latch and closes a notice still on screen
//   (its explanation is no longer true), and a later fallback arms it again.
//
// It is React-free on purpose: `HostNoticeHost` subscribes with
// `useSyncExternalStore`, and the node tests import the store directly. No
// storage is involved, so a restart is a fresh latch.
import type { Translator } from '../i18n.ts';

/** The stable prefix the renderer puts on `cad_status.error` for this fallback. */
export const GPU_FALLBACK_TOKEN = 'gpu.pulled.insufficient-vram';

/** The catalog key of the short banner label for a known token. */
const GPU_FALLBACK_SHORT_KEY = 'gpu.pulled.insufficient_vram.short';

/** Whether `error` is the renderer's pulled-copy fallback report. */
export function isGpuFallbackError(error: string | null | undefined): boolean {
  return typeof error === 'string' && error.startsWith(GPU_FALLBACK_TOKEN);
}

/**
 * The banner's one line: a translated label for a known token, otherwise the
 * host's sentence verbatim. The raw technical string (byte counts and all) stays
 * available as the banner's `title`; only the visible line is translated.
 */
export function statusBannerText(error: string, t: Translator): string {
  if (!isGpuFallbackError(error)) return error;
  const label = t(GPU_FALLBACK_SHORT_KEY);
  // A missing catalog entry resolves to the key itself; the raw host sentence is
  // more honest than showing an identifier in the banner.
  return label === GPU_FALLBACK_SHORT_KEY ? error : label;
}

let tokenPresent = false;
let visible = false;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of [...listeners]) listener();
}

/** Subscribe to visibility changes (the `useSyncExternalStore` contract). */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Whether the modal is currently on screen. */
export function isVisible(): boolean {
  return visible;
}

/** Open the notice. Idempotent; it never stacks a second copy. */
export function showGpuFallbackNotice(): void {
  if (visible) return;
  visible = true;
  emit();
}

/** Dismiss the notice. It stays closed until the token clears and returns. */
export function dismiss(): void {
  if (!visible) return;
  visible = false;
  emit();
}

/**
 * Feed one `cad_status.error` value through the one-shot latch.
 *
 * Called for every status snapshot (not for cursor/FPS frames), so a token that
 * is already present on the first status or after a re-subscribe is noticed even
 * when no `error-raised` event precedes it.
 */
export function observeStatusError(error: string | null | undefined): void {
  if (isGpuFallbackError(error)) {
    if (tokenPresent) return;
    tokenPresent = true;
    showGpuFallbackNotice();
    return;
  }
  if (!tokenPresent && !visible) return;
  tokenPresent = false;
  if (!visible) return;
  visible = false;
  emit();
}

/** Test seam: forget the latch and close the modal. */
export function resetGpuFallbackNoticeForTests(): void {
  tokenPresent = false;
  if (!visible) return;
  visible = false;
  emit();
}
