// Shared React contexts. Regions read the translator and the shell actions
// from here instead of threading a dozen props through every level, so a new
// region only imports `useI18n()` / `useShell()`.
import { createContext, useContext } from 'react';
import type { Dispatch, RefCallback, SetStateAction } from 'react';
import type { Locale, Translator } from './i18n.ts';

/**
 * A host-derived block (`cad_status`, an import progress, a popup payload).
 *
 * `cad.mjs` owns the authoritative shapes and validates them; re-declaring forty
 * host fields here would be a second declaration site that drifts. The regions
 * therefore read these blocks structurally, and this alias is the one documented
 * boundary — it becomes `import type { Status } from './cad.ts'` once that module
 * exports its typed status.
 */
export type ShellRecord = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any -- cad.mjs owns the shape

/** The host `cad_status` block as the regions read it. */
export type ShellStatus = ShellRecord;

/**
 * The i18n context value `App` provides once per document.
 *
 * `t` is the bound translator over the shared catalogs; `setLocale` stages a
 * language choice (persisted by `App`), and `locales` is the shipped set.
 */
export interface I18nValue {
  readonly t: Translator;
  readonly locale: Locale;
  readonly setLocale: Dispatch<SetStateAction<Locale>>;
  readonly locales: readonly string[];
}

/**
 * The shell context value: what `App`'s `shell` object exposes to a region.
 *
 * The state members are the app's own UI state; the action members are the one
 * funnel each kind of affordance uses (`runCommand` for the CAD protocol,
 * `runAction` for chrome-only actions, `runStandardView` for the payload-carrying
 * view IPC). Host-derived blocks stay [`ShellStatus`]/[`ShellRecord`]: `cad.mjs`
 * is their single declaration site.
 */
export interface ShellValue {
  /** The last `cad_status` answer, or `null` before the first one. */
  readonly status: ShellStatus | null;
  /** Whether a command is queued and not yet echoed by the host. */
  readonly pending: boolean;
  /** Whether a document import/render is in flight. */
  readonly loading: boolean;
  /** The host's import progress block, or `null`. */
  readonly importProgress: ShellRecord | null;
  /** Whether the CAD session can admit commands at all. */
  readonly cadReady: boolean;
  /** The open drawing's label, or `null` on the start page. */
  readonly documentLabel: string | null;
  /** Whether a drawing is open (the session fact). */
  readonly documentOpen: boolean;
  /** Whether a drawing is actually on screen (the layout fact). */
  readonly documentVisible: boolean;
  /** Whether this carrier renders the docked palettes. */
  readonly palettesAvailable: boolean;
  /** Whether the host reports a live dialog. */
  readonly popupOpen: boolean;
  /** The rendered frame-rate label (`render.fps.unavailable` when unknown). */
  readonly fpsText: string;
  /** The error the banner shows, or `''`. */
  readonly displayedError: string;
  /** The same banner line (the status bar reads this name). */
  readonly errorText: string;
  /** The last command/probing error, or `''`. */
  readonly error: string;
  /** A polling failure, or `''`. */
  readonly pollError: string;
  /** The typed command history, oldest first. */
  readonly history: string[];
  /** The command line's draft. */
  readonly command: string;
  readonly setCommand: Dispatch<SetStateAction<string>>;
  /** The last `cad_command` answer, or `null`. */
  readonly commandResult: ShellRecord | null;
  /** The message under the command line, or `null`. */
  readonly commandMessage: ShellRecord | null;
  /** The host's alias table (`cad_command_aliases`), suggestion metadata only. */
  readonly commandAliases: readonly ShellRecord[];
  /** Submit typed command text through the one funnel. */
  readonly submitCommand: (text: string, options?: ShellRecord) => Promise<void>;
  /** Submit the active text-carrying tool's text. */
  readonly submitToolText: (text: string) => Promise<ShellRecord>;
  /** Apply text to the current single selection. */
  readonly applySelectedText: (text: string) => Promise<ShellRecord>;
  /** Queue one property write for the current selection. */
  readonly applySelectedProperty: (key: string, value: string) => Promise<ShellRecord>;
  /** Open the raw-text dialog for the current single selection. */
  readonly openTextEditPopup: () => void;
  /** Open (or focus) the Options dialog. */
  readonly openOptionsPopup: () => void;
  readonly historyOpen: boolean;
  readonly setHistoryOpen: Dispatch<SetStateAction<boolean>>;
  readonly clearHistory: () => void;
  /** The selected ribbon tab id. */
  readonly ribbonTab: string;
  readonly setRibbonTab: Dispatch<SetStateAction<string>>;
  readonly ribbonCollapsed: boolean;
  readonly setRibbonCollapsed: Dispatch<SetStateAction<boolean>>;
  readonly cleanScreen: boolean;
  readonly setCleanScreen: Dispatch<SetStateAction<boolean>>;
  readonly diagnosticsOpen: boolean;
  readonly setDiagnosticsOpen: Dispatch<SetStateAction<boolean>>;
  readonly propertiesOpen: boolean;
  readonly setPropertiesOpen: Dispatch<SetStateAction<boolean>>;
  readonly layersOpen: boolean;
  readonly setLayersOpen: Dispatch<SetStateAction<boolean>>;
  readonly toggleLayers: () => void;
  /** Bumped by every `LAYER`: an open palette re-reads and takes the keyboard. */
  readonly layersFocus: number;
  readonly commandLineOpen: boolean;
  readonly setCommandLineOpen: Dispatch<SetStateAction<boolean>>;
  /** The CAD surface node's ref callback (also used to restore focus). */
  readonly surfaceRef: RefCallback<HTMLElement>;
  /** Whether a router command may run right now. */
  readonly canCommand: (command: string) => boolean;
  /** Run one payload-free router command (`cad_command`). */
  readonly runCommand: (command: string) => void;
  /** Run one payload-carrying standard view (`cad_standard_view`). */
  readonly runStandardView: (view: string) => void;
  /** Ask the host to cancel an in-flight import. */
  readonly cancelImport: () => void;
  /** The active numeric parameter's bounded pending delta, or `null`. */
  readonly toolParameterPending: ShellRecord | null;
  /** One numeric parameter step (the pending delta absorbs it first). */
  readonly stepToolParameter: (key: string, target: number) => void;
  /** One flag parameter toggle. */
  readonly toggleToolParameter: (key: string, next: boolean) => void;
  /** Run one DOM-only shell action. */
  readonly runAction: (action: string) => void;
  /** Suspend/resume the CAD pointer lane (the chrome owns the keyboard). */
  readonly suspendPointer: (suspended: boolean) => void;
}

export const I18nContext = createContext<I18nValue | null>(null);
export const ShellContext = createContext<ShellValue | null>(null);

export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error('useI18n must be used inside <I18nContext.Provider>');
  return value;
}

export function useShell(): ShellValue {
  const value = useContext(ShellContext);
  if (!value) throw new Error('useShell must be used inside <ShellContext.Provider>');
  return value;
}
