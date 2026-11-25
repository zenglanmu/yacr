// The plugin-facing UI contribution seam.
//
// A same-privilege plugin module may import this module and register a
// **declarative** patch: menus, tool groups and tray panels built from the same
// `RibbonTab`/`RibbonPanel`/`RibbonItem` data the chrome uses. It cannot inject
// JSX or a second React root, and every command/action still goes through the
// shared admission, so a patch cannot invent an accepted command.
//
// This is deliberately **not** advertised as a `cad-plugin-api` namespace yet:
// `packages/cad-plugin-api` and the host descriptor still describe commands and
// document reads only. Adding `ui.register` to that public contract (and the
// host capability JSON) is the next step; until then a plugin that needs the
// slot seam imports this module directly, which is exactly the same-privilege
// model ADR-0029/0030/0034 already use. Do not claim a capability the host does
// not advertise.
export { registerPluginUi, unregisterPluginUi } from '../domains/ui/registry.ts';
export type { DomainUiPatch, DomainUiLayout, TrayPanel } from '../domains/ui/slots.ts';
