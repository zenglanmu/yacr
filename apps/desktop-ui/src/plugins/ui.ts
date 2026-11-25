// The plugin-facing UI contribution seam (host side).
//
// The public authoring surface is `@yacr/plugin-api`'s `api.ui.register`, which
// hands a **declarative** contribution to the loader; `apps/desktop-ui/src/plugin-api/ui.ts`
// maps it onto `registerPluginUi`. This module stays the host-side registry entry
// for same-privilege modules that already import host source, and it still cannot
// inject JSX or a second React root: a contribution is data, and every command
// item goes through the shared admission, so a patch cannot invent an accepted
// command.
//
// Do not describe `ui.register` as a descriptor operation: it is client-local,
// exactly like `runtime.registerCommand`, and has no row in
// `crates/cad-ui-core/api/plugin-api.json`.
export { registerPluginUi, unregisterPluginUi } from '../domains/ui/registry.ts';
export type { DomainUiPatch, DomainUiLayout, TrayPanel } from '../domains/ui/slots.ts';
