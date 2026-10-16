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
