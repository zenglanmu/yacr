// The two message catalogs, compiled into the bundle once.
//
// Both the main shell and a host-opened popup window (same bundle, second
// document) translate from these, so a dialog can never drift onto a private
// copy. The files themselves are the shared, platform-independent authority
// under `crates/cad-ui-core/i18n/` (injected by the Vite `@i18n` alias).
import zh from '@i18n/zh-CN.json';
import en from '@i18n/en.json';

/** One catalog: a flat `key -> message` table, exactly as the JSON files are written. */
export type Catalog = Readonly<Record<string, string>>;

/**
 * The shipped catalogs, keyed by the only locales this build supports.
 *
 * A lookup by an unsupported locale is `undefined`, which is what
 * `catalogFor` in `i18n.ts` turns into the English fallback.
 */
export const CATALOGS: Readonly<Record<string, Catalog>> = Object.freeze({ 'zh-CN': zh, en });
