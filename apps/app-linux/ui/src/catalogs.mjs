// The two message catalogs, compiled into the bundle once.
//
// Both the main shell and a host-opened popup window (same bundle, second
// document) translate from these, so a dialog can never drift onto a private
// copy. The files themselves are the shared, platform-independent authority
// under `crates/cad-ui-core/i18n/` (injected by the Vite `@i18n` alias).
import zh from '@i18n/zh-CN.json';
import en from '@i18n/en.json';

export const CATALOGS = Object.freeze({ 'zh-CN': zh, en });
