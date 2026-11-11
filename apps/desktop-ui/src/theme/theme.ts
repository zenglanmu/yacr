// The UI theme: one host-reported value, one document attribute, one cached
// local mirror.
//
// `cad_status.theme` is the authority. The chrome never flips its own copy on a
// click: a toggle writes the opposite value through the same `cad_set_setting`
// seam the Options dialog uses (`writeSetting`), and the next host status applies
// it. The localStorage entry is only a **first-paint cache** of the last value
// the host confirmed, so a restart does not flash the other theme before the
// first status frame; a blocked storage (a locked WebView) is a no-op, never an
// error.
//
// The document attribute is `data-theme` on `<html>`, written here and by
// `main.tsx` before React mounts (first paint). The stylesheet keys every light
// override on `:root[data-theme="light"]`, so no component has to know a colour.
import { useLayoutEffect } from 'react';
import { invoke } from '../host/invoke.ts';
import { writeSetting } from '../options/options.ts';
import type { SettingWriteResult } from '../options/options.ts';

/** Guarded storage key: the last host-confirmed theme. */
export const THEME_STORAGE_KEY = 'yacr.theme';

/** The one setting key the toggle writes (`cad_set_setting { key, value }`). */
export const THEME_SETTING_KEY = 'theme';

/** The two documented UI themes. */
export type Theme = 'dark' | 'light';

/** The document attribute the stylesheet keys on. */
export const THEME_ATTRIBUTE = 'data-theme';

/** The historical chrome: the value an older host and a first run start in. */
export const DEFAULT_THEME: Theme = 'dark';

/**
 * Whether a boot must replay the cached theme into the host.
 *
 * The native carriers persist `theme` in `preference.json`; the `dom-canvas`
 * session's settings store is in-memory, so a page reload starts from the host
 * default while the cache still holds the user's last choice. Only that carrier
 * (and only a real disagreement) replays the cache through `cad_set_setting`;
 * a native host is never overridden by a stale local mirror.
 */
export function shouldReplayStoredTheme(
  hostTheme: unknown,
  surfaceMode: string | undefined,
  cached: Theme | null,
): boolean {
  const host = normalizeTheme(hostTheme);
  return host !== null
    && cached !== null
    && host !== cached
    && surfaceMode === 'dom-canvas';
}

/** One of the two names, or `null` for anything else (never a guessed theme). */
export function normalizeTheme(value: unknown): Theme | null {
  return value === 'dark' || value === 'light' ? value : null;
}

/** The other theme (the toggle's target). */
export function oppositeTheme(theme: Theme): Theme {
  return theme === 'light' ? 'dark' : 'light';
}

/** `window.localStorage`, or `null` where the engine blocks or omits it. */
function storage(): Storage | null {
  if (typeof window === 'undefined') return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/** The cached theme, or `null` when absent/unreadable/not a known name. */
export function storedTheme(): Theme | null {
  const store = storage();
  if (!store) return null;
  try {
    return normalizeTheme(store.getItem(THEME_STORAGE_KEY));
  } catch {
    return null;
  }
}

/** Mirror a host-confirmed theme for the next first paint. A blocked write is a no-op. */
export function storeTheme(theme: Theme): void {
  const store = storage();
  if (!store) return;
  try {
    store.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // A read-only/locked storage must not break the shell: the host stays the
    // authority and the next status still applies the theme.
  }
}

/** The theme the document should paint before the first host status arrives. */
export function initialTheme(): Theme {
  return storedTheme() ?? DEFAULT_THEME;
}

/**
 * Apply one theme to `<html>`.
 *
 * `data-theme` is the single stylesheet switch; the `color-scheme` declaration
 * lives beside the light token block in `style.css`.
 */
export function applyTheme(theme: Theme): void {
  if (typeof document === 'undefined') return;
  document.documentElement.dataset.theme = theme;
}

/**
 * Follow the host theme and mirror it to the first-paint cache.
 *
 * A status without a theme (a recorded pre-theme payload, a test double) keeps
 * the cached value already painted by `main.tsx`; it never invents a switch. A
 * `useLayoutEffect` applies a newly reported theme before the browser paints, so
 * a host that starts in light does not flash dark chrome.
 *
 * On a `dom-canvas` carrier (whose settings store is in-memory) the last
 * host-confirmed theme is replayed once after the first status, so a page reload
 * keeps the user's choice instead of silently reverting to the default.
 */
let domCanvasThemeReplayed = false;

export function useHostTheme(hostTheme: unknown, surfaceMode?: string): Theme {
  const host = normalizeTheme(hostTheme);
  const theme = host ?? initialTheme();
  useLayoutEffect(() => {
    if (!host) return;
    const cached = storedTheme();
    applyTheme(host);
    storeTheme(host);
    if (!domCanvasThemeReplayed && cached !== null
      && shouldReplayStoredTheme(host, surfaceMode, cached)) {
      domCanvasThemeReplayed = true;
      // Queued through the same settings seam; the next status confirms it. A
      // refusal leaves the host value in place (the UI already follows it).
      void setTheme(cached);
    }
  }, [host, surfaceMode]);
  return theme;
}

/**
 * Ask the host for one theme change.
 *
 * Admitted (`accepted`) means **queued**, exactly like every other setting
 * write; the interface changes only when `cad_status.theme` reports the new
 * value. A refusal leaves both the document and the cache on the host's current
 * value and returns the stable error code for the caller to show.
 */
export function setTheme(theme: Theme): Promise<SettingWriteResult> {
  return writeSetting(invoke, THEME_SETTING_KEY, theme);
}
