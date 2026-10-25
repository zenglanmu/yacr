// Message catalog access for the React chrome.
//
// Every user-visible string goes through `t(key, params)`. Catalogs are the
// shared, platform-independent JSON files under `crates/cad-ui-core/i18n/`
// (injected by the Vite `@i18n` alias). Keys are never invented here: a missing
// key resolves to the key itself so a gap is visible instead of silently empty.

/** One catalog: a flat `key -> message` table. */
export type Catalog = Readonly<Record<string, string>>;

/** Every catalog this build ships, in menu order. */
export type Locale = 'zh-CN' | 'en';

/** A lookup table of catalogs by locale tag; a locale it omits has no catalog. */
export type CatalogMap = Readonly<Record<string, Catalog | undefined>>;

/** Placeholder values for one message (`{name}` → `params.name`). */
export type TranslateParams = Readonly<Record<string, unknown>>;

/** A bound translator: `t(key)` / `t(key, params)`. */
export type Translator = (key: string, params?: TranslateParams | null) => string;

/**
 * The navigator fields the locale resolution reads. Structural, so a real
 * `Navigator` and the plain objects the tests pass both fit.
 */
export interface NavigatorLike {
  readonly languages?: readonly string[];
  readonly language?: string;
}

/** Why `resolveLocale` picked its locale. */
export type LocaleSource = 'stored' | 'system' | 'fallback';

/** The chosen locale plus the source that decided it. */
export interface ResolvedLocale {
  readonly locale: Locale;
  readonly source: LocaleSource;
}

export const LOCALES: readonly Locale[] = ['zh-CN', 'en'];
/**
 * Catalog used when no supported locale can be resolved. English, not Chinese:
 * a system language this build does not ship must not silently land on a
 * language the user did not ask for.
 */
export const DEFAULT_LOCALE: Locale = 'en';

/** Pick a catalog for `locale`, falling back to the default locale. */
export function catalogFor(locale: string | null | undefined, catalogs: CatalogMap | null | undefined): Catalog {
  return catalogs?.[typeof locale === 'string' ? locale : ''] ?? catalogs?.[DEFAULT_LOCALE] ?? {};
}

/** Replace `{name}` placeholders; an unknown placeholder is left untouched. */
export function formatMessage(template: unknown, params?: TranslateParams | null): string {
  if (typeof template !== 'string') return '';
  if (!params) return template;
  return template.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) && params[name] != null
      ? String(params[name])
      : match);
}

/**
 * Build a translator over `catalogs`. `locale` selects the primary catalog;
 * `fallbackLocale` is consulted when a key is missing there, so a partially
 * translated locale never blanks out the chrome.
 */
export function createTranslator(
  catalogs: CatalogMap | null | undefined,
  locale: string | null | undefined,
  fallbackLocale = DEFAULT_LOCALE,
): Translator {
  const primary = catalogFor(locale, catalogs);
  const fallback = catalogFor(fallbackLocale, catalogs);
  return function t(key: string, params?: TranslateParams | null): string {
    const template = primary[key] ?? fallback[key];
    if (template === undefined) return key;
    return formatMessage(template, params);
  };
}

/** Locale tag for `document.documentElement.lang`. */
export function localeTag(locale: string | null | undefined): Locale {
  return isLocale(locale) ? locale : DEFAULT_LOCALE;
}

/** Whether `value` is one of the locales this build ships a catalog for. */
function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && (LOCALES as readonly string[]).includes(value);
}

/**
 * The locale the document should start in, and why it was chosen.
 *
 * Order (highest first): the persisted explicit choice (`stored`), then the tag
 * the **host** resolved and injected before page load (`system`), then the
 * OS/WebView UI language (`system`), then English (`fallback`).
 *
 * The host tag is what keeps the chrome and the native dialog labels in the same
 * language: it is the already-resolved tag Rust uses for those labels, so it is
 * authoritative over `navigator` even when the two inputs disagree. When it maps
 * to no catalog (e.g. `zh-TW`, `fr`) the answer is English, not a fall-through
 * to `navigator` — otherwise a Traditional-Chinese system with a `zh-CN`
 * `navigator` would show Simplified chrome next to English native labels.
 *
 * `document.documentElement.lang` is deliberately **not** a source: it is a
 * static file value, so reading it would lock the default at build time. The
 * caller writes it from the resolved locale instead.
 *
 * **Behaviour change:** the language choice is persisted in `localStorage`
 * (key [`LOCALE_STORAGE_KEY`]) and survives a restart. It is the same origin
 * for the shell and for a popup window, so both documents read the same value —
 * that is also what carries a runtime switch into a dialog.
 *
 * Storage is only ever a hint: an absent, blocked, throwing or unrecognised
 * value falls through to the next source, so a corrupt entry can never break
 * boot or select an unsupported locale.
 */
export function resolveLocale(
  doc: Document | null | undefined = typeof document === 'undefined' ? null : document,
  navigatorObject: NavigatorLike | null | undefined = typeof navigator === 'undefined' ? null : navigator,
  hostLocale: string | undefined = injectedHostLocale(),
): ResolvedLocale {
  const stored = storedLocale(doc);
  if (stored) return { locale: stored, source: 'stored' };
  if (hostLocale !== undefined) {
    // The host already resolved and normalised this tag. An unmappable value is
    // English (`en`), never a fall-through to `navigator`.
    return { locale: systemLocaleFor(hostLocale) ?? DEFAULT_LOCALE, source: 'system' };
  }
  for (const tag of systemLanguageTags(navigatorObject)) {
    const mapped = systemLocaleFor(tag);
    if (mapped) return { locale: mapped, source: 'system' };
  }
  return { locale: DEFAULT_LOCALE, source: 'fallback' };
}

/**
 * The locale the document should start in (see [`resolveLocale`]).
 */
export function initialLocale(
  doc?: Document | null,
  navigatorObject?: NavigatorLike | null,
  hostLocale?: string,
): Locale {
  return resolveLocale(doc, navigatorObject, hostLocale).locale;
}

declare global {
  interface Window {
    /** The host's resolved locale tag, injected before the document is parsed. */
    readonly __YACR_HOST_LOCALE__?: unknown;
  }
}

/**
 * Global the host sets (with an `initialization_script`, before the document is
 * parsed) to its resolved locale tag. Absent in a plain browser and in tests.
 */
export const HOST_LOCALE_GLOBAL = '__YACR_HOST_LOCALE__';

/**
 * The locale tag the host injected, or `undefined` when this document is not
 * hosted (no `window`, or the global is not a string).
 */
export function injectedHostLocale(): string | undefined {
  if (typeof window === 'undefined') return undefined;
  const tag = window[HOST_LOCALE_GLOBAL];
  return typeof tag === 'string' ? tag : undefined;
}

/**
 * Map a system locale tag onto a shipped catalog, or `null` when this build has
 * none (which resolves to English).
 *
 * This is the same table as Rust `cad_ui_core::i18n::supported_locale_for`.
 * `en`/`en-*` select English; `zh`, `zh-CN`, `zh-Hans` and other
 * Simplified-Chinese regions select `zh-CN`. A Hant script or a Traditional
 * region (`zh-TW`, `zh-HK`, `zh-MO`) selects `null`: no Hant catalog exists, so
 * it is not silently served Simplified. Codeset/modifier suffixes
 * (`zh_TW.UTF-8`, `sr_RS@latin`) are stripped first.
 */
export function systemLocaleFor(tag: unknown): Locale | null {
  if (typeof tag !== 'string') return null;
  const base = tag.trim().split(/[.@]/)[0];
  const parts = base.split(/[-_]/);
  const primary = (parts[0] ?? '').toLowerCase();
  if (!/^[a-z]+$/.test(primary)) return null;
  if (primary === 'en') return 'en';
  if (primary !== 'zh') return null;
  const traditional = parts
    .slice(1)
    .some(part => ['hant', 'tw', 'hk', 'mo'].includes(part.toLowerCase()));
  return traditional ? null : 'zh-CN';
}

/** The OS/WebView UI languages in preference order (`languages`, then `language`). */
function systemLanguageTags(navigatorObject: NavigatorLike | null | undefined): readonly string[] {
  const languages = navigatorObject?.languages;
  if (Array.isArray(languages) && languages.length > 0) return languages;
  const language = navigatorObject?.language;
  return typeof language === 'string' && language ? [language] : [];
}

/** `localStorage` key holding the chosen locale (same origin for every window). */
export const LOCALE_STORAGE_KEY = 'yacr.locale';

function localeStorage(doc: Document | null | undefined): Storage | null {
  try {
    return doc?.defaultView?.localStorage ?? null;
  } catch {
    // Some privacy modes throw on the property itself, not just on access.
    return null;
  }
}

/**
 * The persisted locale, when it is one this build supports; otherwise `null`.
 *
 * Validation is strict membership in [`LOCALES`] — a garbage value must not
 * leak into `createTranslator`, where it would silently select the default
 * catalog without anyone noticing the storage was corrupt.
 */
export function storedLocale(doc: Document | null | undefined = typeof document === 'undefined' ? null : document): Locale | null {
  try {
    const raw = localeStorage(doc)?.getItem(LOCALE_STORAGE_KEY);
    return isLocale(raw) ? raw : null;
  } catch {
    return null;
  }
}

/**
 * Persist `locale` for the next document/restart.
 *
 * Returns whether the value was written; a blocked storage is a silent no-op
 * (the session still honours the in-memory choice), never an error banner.
 */
export function storeLocale(
  locale: string | null | undefined,
  doc: Document | null | undefined = typeof document === 'undefined' ? null : document,
): boolean {
  if (!isLocale(locale)) return false;
  try {
    const storage = localeStorage(doc);
    if (!storage) return false;
    storage.setItem(LOCALE_STORAGE_KEY, locale);
    return true;
  } catch {
    return false;
  }
}
