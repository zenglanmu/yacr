// The domain/plugin UI registry: domain layouts plus additive plugin patches.
//
// Precedence is static-domain first, then plugin patches in registration order.
// A patch can add a menu/group/panel or extend an existing one by id, so a
// plugin never has to fork a whole layout to add one tool. A patch can also
// target `'*'`, which is how an extension ships one toolbar button to every
// application domain.
//
// Registration is same-privilege interop glue (like the vendor plugin host): it
// validates shape and ids, but it is not a sandbox. Command/action dispatch
// still passes the shared admission (`runItem` → `cad_command`/`runAction`), so a
// patch cannot invent an accepted command.
//
// The store is deliberately tiny: a revision counter plus `subscribe`, read by
// `useDomainUi`. Layouts are merged on read and frozen, which means a plugin may
// register at any time and every mounted slot sees the same next snapshot.
import { DOMAIN_IDS } from '../domains.ts';
import type { DomainId } from '../domains.ts';
import type { RibbonItem, RibbonPanel, RibbonTab } from '../../ribbon/ribbon-model.ts';
import type { DomainUiLayout, DomainUiPatch, TrayPanel } from './slots.ts';

/** Stable error prefix for a malformed registration or patch. */
const ERROR = 'domain.ui';

let revision = 0;
const listeners = new Set<() => void>();
const DOMAINS = new Map<DomainId, DomainUiLayout>();
const PATCHES = new Map<string, DomainUiPatch>();

function fail(detail: string): never {
  throw new Error(`${ERROR}: ${detail}`);
}

function bump(): void {
  revision += 1;
  for (const listener of [...listeners]) listener();
}

/** Subscribe to registry changes; returns the unsubscribe function. */
export function subscribeDomainUi(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** The current revision (the `useSyncExternalStore` snapshot). */
export function domainUiRevision(): number {
  return revision;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isKnownDomain(value: unknown): value is DomainId {
  return typeof value === 'string' && (DOMAIN_IDS as readonly string[]).includes(value);
}

function nonEmpty(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

function validItem(value: unknown): value is RibbonItem {
  if (!isRecord(value)) return false;
  if (!nonEmpty(value['id']) || !nonEmpty(value['labelKey']) || !nonEmpty(value['icon'])) return false;
  // Plugin contributions ship literal text because they cannot add catalog rows;
  // host-owned items omit it and keep resolving through `labelKey`.
  if (value['label'] !== undefined && !nonEmpty(value['label'])) return false;
  if (value['pendingText'] !== undefined && !nonEmpty(value['pendingText'])) return false;
  // Exactly one dispatch token, or none for a labelled pending item that names
  // its own sentence. Two tokens would make dispatch order undefined.
  const tokens = ['command', 'standardView', 'viewPreset', 'action'].filter(key => nonEmpty(value[key]));
  if (tokens.length > 1) return false;
  if (tokens.length === 0 && !nonEmpty(value['pendingKey'])) return false;
  if (value['variants'] !== undefined) {
    if (!Array.isArray(value['variants']) || !value['variants'].every(validItem)) return false;
  }
  return true;
}

function validPanel(value: unknown): value is RibbonPanel {
  if (!isRecord(value)) return false;
  if (!nonEmpty(value['id']) || !nonEmpty(value['titleKey'])) return false;
  if (value['title'] !== undefined && !nonEmpty(value['title'])) return false;
  if (!Array.isArray(value['items'])) return false;
  // A pending panel is documented as empty; a live panel needs at least one
  // item. Both are checked so an extension cannot ship a titled empty box.
  if (value['pending'] === true) return value['items'].length === 0;
  return value['items'].length > 0 && value['items'].every(validItem);
}

function validTab(value: unknown): value is RibbonTab {
  if (!isRecord(value)) return false;
  if (!nonEmpty(value['id']) || !nonEmpty(value['labelKey'])) return false;
  if (value['label'] !== undefined && !nonEmpty(value['label'])) return false;
  return Array.isArray(value['panels']) && value['panels'].every(validPanel);
}

function validTrayPanel(value: unknown): value is TrayPanel {
  if (!isRecord(value)) return false;
  if (!nonEmpty(value['id']) || !nonEmpty(value['titleKey'])) return false;
  const kind = value['kind'];
  if (typeof kind !== 'string') return false;
  const kinds = ['entityInfo', 'components', 'styles', 'tags', 'scenes', 'instructor', 'bimQuery', 'mechanicalTree', 'materials', 'soften', 'pending'];
  if (!kinds.includes(kind)) return false;
  if (value['title'] !== undefined && !nonEmpty(value['title'])) return false;
  if (value['pendingText'] !== undefined && !nonEmpty(value['pendingText'])) return false;
  // Only the pending body needs a sentence; a real body carries its own copy.
  if (kind === 'pending') return nonEmpty(value['pendingKey']);
  return value['pendingKey'] === undefined;
}

function validateLayout(value: unknown, label: string): DomainUiLayout {
  if (!isRecord(value)) fail(`${label} must be an object`);
  if (value['ribbon'] !== undefined) {
    if (!Array.isArray(value['ribbon']) || !value['ribbon'].every(validTab)) fail(`${label}.ribbon`);
  }
  if (value['menu'] !== undefined) {
    if (!Array.isArray(value['menu']) || !value['menu'].every(validTab)) fail(`${label}.menu`);
  }
  if (value['toolbar'] !== undefined) {
    if (!Array.isArray(value['toolbar']) || !value['toolbar'].every(validPanel)) fail(`${label}.toolbar`);
  }
  if (value['tray'] !== undefined) {
    if (!Array.isArray(value['tray']) || !value['tray'].every(validTrayPanel)) fail(`${label}.tray`);
  }
  return value as DomainUiLayout;
}

function freezePanels(panels: readonly RibbonPanel[]): readonly RibbonPanel[] {
  return Object.freeze(panels.map(panel => Object.freeze({
    ...panel,
    items: Object.freeze([...panel.items]),
  })));
}

function freezeTabs(tabs: readonly RibbonTab[]): readonly RibbonTab[] {
  return Object.freeze(tabs.map(tab => Object.freeze({
    ...tab,
    panels: freezePanels(tab.panels),
  })));
}

/**
 * Register one domain's layout.
 *
 * Re-registering the same domain replaces it (the last module to run wins),
 * which is how a test or an application-specific build overrides a profile.
 */
export function registerDomainUi(domain: DomainId, layout: DomainUiLayout): void {
  if (!isKnownDomain(domain)) fail(`unknown domain: ${String(domain)}`);
  DOMAINS.set(domain, validateLayout(layout, domain));
  bump();
}

/**
 * Register one plugin patch.
 *
 * `id` is the unregistration handle and must be unique; a second registration
 * of the same id is refused (`domain.ui: duplicate patch`) instead of silently
 * replacing a live extension. `domain: '*'` targets every profile.
 */
export function registerPluginUi(patch: DomainUiPatch): void {
  if (!isRecord(patch)) fail('patch must be an object');
  if (!nonEmpty(patch.id) || !/^[A-Za-z0-9][A-Za-z0-9_.:-]*$/.test(patch.id)) fail('patch.id');
  if (PATCHES.has(patch.id)) fail(`duplicate patch: ${patch.id}`);
  if (patch.domain !== '*' && !isKnownDomain(patch.domain)) fail(`patch.domain: ${String(patch.domain)}`);
  validateLayout(patch, `patch ${patch.id}`);
  if (patch.ribbon === undefined && patch.menu === undefined && patch.toolbar === undefined && patch.tray === undefined) {
    fail(`patch ${patch.id} contributes no slot`);
  }
  PATCHES.set(patch.id, { ...patch, ...(patch.menu ? { menu: [...patch.menu] } : {}) });
  bump();
}

/** Remove one plugin patch. An unknown id is a no-op (deactivation is idempotent). */
export function unregisterPluginUi(id: string): boolean {
  const removed = PATCHES.delete(id);
  if (removed) bump();
  return removed;
}

function mergePanels(base: readonly RibbonPanel[], patch: readonly RibbonPanel[]): RibbonPanel[] {
  const out = [...base];
  for (const panel of patch) {
    const at = out.findIndex(entry => entry.id === panel.id);
    if (at === -1) {
      out.push(panel);
      continue;
    }
    // Extending a panel: append items that are new by id, keep the base order
    // otherwise. The base wins on an id collision: a later patch cannot shadow a
    // domain's own row (two rows with one id would also make React keys collide).
    const seen = new Set(out[at].items.map(item => item.id));
    const extra = panel.items.filter(item => {
      if (seen.has(item.id)) return false;
      seen.add(item.id);
      return true;
    });
    out[at] = { ...out[at], items: [...out[at].items, ...extra] };
  }
  return out;
}

function mergeTabs(base: readonly RibbonTab[], patch: readonly RibbonTab[]): RibbonTab[] {
  const out = [...base];
  for (const tab of patch) {
    const at = out.findIndex(entry => entry.id === tab.id);
    if (at === -1) {
      out.push(tab);
      continue;
    }
    out[at] = { ...out[at], panels: mergePanels(out[at].panels, tab.panels) };
  }
  return out;
}

function mergeTray(base: readonly TrayPanel[], patch: readonly TrayPanel[]): TrayPanel[] {
  const out = [...base];
  for (const panel of patch) {
    const at = out.findIndex(entry => entry.id === panel.id);
    if (at === -1) out.push(panel);
    else out[at] = panel;
  }
  return out;
}

/**
 * The effective layout for one domain, frozen.
 *
 * Merging is pure and runs per read, so the caller can memoise on
 * [`domainUiRevision`]. Absent slots stay `undefined`: the slot renderer then
 * draws nothing rather than an empty band.
 */
export function uiLayoutFor(domain: DomainId): DomainUiLayout {
  const base = DOMAINS.get(domain) ?? {};
  let ribbon = base.ribbon ? [...base.ribbon] : undefined;
  let menu = base.menu ? [...base.menu] : undefined;
  let toolbar = base.toolbar ? [...base.toolbar] : undefined;
  let tray = base.tray ? [...base.tray] : undefined;
  for (const patch of PATCHES.values()) {
    if (patch.domain !== '*' && patch.domain !== domain) continue;
    if (patch.ribbon) ribbon = mergeTabs(ribbon ?? [], patch.ribbon);
    if (patch.menu) menu = mergeTabs(menu ?? [], patch.menu);
    if (patch.toolbar) toolbar = mergePanels(toolbar ?? [], patch.toolbar);
    if (patch.tray) tray = mergeTray(tray ?? [], patch.tray);
  }
  return Object.freeze({
    ...(ribbon ? { ribbon: freezeTabs(ribbon) } : {}),
    ...(menu ? { menu: freezeTabs(menu) } : {}),
    ...(toolbar ? { toolbar: freezePanels(toolbar) } : {}),
    ...(tray ? { tray: Object.freeze([...tray]) } : {}),
  });
}

/** Test seam: forget every domain layout and patch. */
export function resetDomainUi(): void {
  DOMAINS.clear();
  PATCHES.clear();
  bump();
}

/** The ids of every registered plugin patch (diagnostics/tests). */
export function pluginUiIds(): readonly string[] {
  return [...PATCHES.keys()];
}
