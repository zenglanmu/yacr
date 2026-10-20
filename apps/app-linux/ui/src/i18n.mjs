// Message catalog access for the React chrome.
//
// Every user-visible string goes through `t(key, params)`. Catalogs are the
// shared, platform-independent JSON files under `crates/cad-ui-core/i18n/`
// (injected by the Vite `@i18n` alias). Keys are never invented here: a missing
// key resolves to the key itself so a gap is visible instead of silently empty.

export const LOCALES = ['zh-CN', 'en'];
export const DEFAULT_LOCALE = 'zh-CN';

/** Pick a catalog for `locale`, falling back to the default locale. */
export function catalogFor(locale, catalogs) {
  return catalogs?.[locale] ?? catalogs?.[DEFAULT_LOCALE] ?? {};
}

/** Replace `{name}` placeholders; an unknown placeholder is left untouched. */
export function formatMessage(template, params) {
  if (typeof template !== 'string') return '';
  if (!params) return template;
  return template.replace(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g, (match, name) =>
    Object.prototype.hasOwnProperty.call(params, name) && params[name] != null
      ? String(params[name])
      : match);
}

/**
 * Build a translator over `catalogs`. `locale` selects the primary catalog;
 * `fallbackLocale` is consulted when a key is missing there, so a partially
 * translated locale never blanks out the chrome.
 */
export function createTranslator(catalogs, locale, fallbackLocale = DEFAULT_LOCALE) {
  const primary = catalogFor(locale, catalogs);
  const fallback = catalogFor(fallbackLocale, catalogs);
  return function t(key, params) {
    const template = primary[key] ?? fallback[key];
    if (template === undefined) return key;
    return formatMessage(template, params);
  };
}

/** Locale tag for `document.documentElement.lang`. */
export function localeTag(locale) {
  return LOCALES.includes(locale) ? locale : DEFAULT_LOCALE;
}

/**
 * The locale the document should start in.
 *
 * Order: the persisted choice, then the document's declared `lang`, then the
 * default. **Behaviour change:** the language choice is now persisted in
 * `localStorage` (key [`LOCALE_STORAGE_KEY`]) and survives a restart. It is the
 * same origin for the shell and for a popup window, so both documents read the
 * same value — that is also what carries a runtime switch into a dialog.
 *
 * Storage is only ever a hint: an absent, blocked, throwing or unrecognised
 * value falls through to the next source, so a corrupt entry can never break
 * boot or select an unsupported locale.
 */
export function initialLocale(doc = typeof document === 'undefined' ? null : document) {
  const stored = storedLocale(doc);
  if (stored) return stored;
  const declared = doc?.documentElement?.lang ?? '';
  return LOCALES.includes(declared) ? declared : DEFAULT_LOCALE;
}

/** `localStorage` key holding the chosen locale (same origin for every window). */
export const LOCALE_STORAGE_KEY = 'yacr.locale';

function localeStorage(doc) {
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
export function storedLocale(doc = typeof document === 'undefined' ? null : document) {
  try {
    const raw = localeStorage(doc)?.getItem(LOCALE_STORAGE_KEY);
    return LOCALES.includes(raw) ? raw : null;
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
export function storeLocale(locale, doc = typeof document === 'undefined' ? null : document) {
  if (!LOCALES.includes(locale)) return false;
  try {
    const storage = localeStorage(doc);
    if (!storage) return false;
    storage.setItem(LOCALE_STORAGE_KEY, locale);
    return true;
  } catch {
    return false;
  }
}
