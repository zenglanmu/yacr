// The WebView adapter for `@yacr/plugin-api`'s client-local `ui` namespace.
//
// The package owns the host-agnostic contribution shape; this module is the only
// place that knows the host's slots (`RibbonTab`/`RibbonPanel`/`TrayPanel`). It
// maps one validated contribution onto the existing `DomainUiPatch` registry, so
// plugin UI travels the same merge path and the same dispatch funnel as the
// domain's own chrome.
//
// A plugin cannot add catalog rows, so the package ships literal text. The host
// model carries it as `label`/`title`/`pendingText`, which win over the catalog
// lookup; the synthetic key stays a stable, never-displayed identifier (and is
// what the domain registry's shape validation checks).
//
// This is interop glue, not a security boundary: the registry still refuses
// malformed ids, unknown domains and duplicate patches, and every command item
// is admitted by `canCommand`.
import { CadApiError } from '../../../../packages/cad-plugin-api/src/index.ts';
import type { UiRegistration, UiItem } from '../../../../packages/cad-plugin-api/src/index.ts';
import { DOMAIN_IDS } from '../domains/domains.ts';
import type { DomainId } from '../domains/domains.ts';
import { registerPluginUi, unregisterPluginUi } from '../domains/ui/registry.ts';
import type { DomainUiPatch, TrayPanel } from '../domains/ui/slots.ts';
import type { RibbonItem, RibbonPanel, RibbonTab } from '../ribbon/ribbon-model.ts';

/** One stable synthetic catalog key. It is never shown; literal text wins. */
function key(contributionId: string, slot: string, id: string): string {
  return `plugin.${contributionId}.${slot}.${id}`;
}

function itemFor(contributionId: string, slot: string, item: UiItem): RibbonItem {
  const labelKey = key(contributionId, slot, item.id);
  if (item.pendingReason !== undefined) {
    return {
      id: item.id,
      labelKey,
      label: item.label,
      icon: item.icon ?? 'dot',
      pendingKey: `${labelKey}.pending`,
      pendingText: item.pendingReason,
    };
  }
  return {
    id: item.id,
    labelKey,
    label: item.label,
    icon: item.icon ?? 'dot',
    command: item.command as string,
  };
}

function panelsFor(contributionId: string, slot: string, groups: UiRegistration['toolbar']): readonly RibbonPanel[] {
  return (groups ?? []).map(group => ({
    id: group.id,
    titleKey: key(contributionId, slot, group.id),
    title: group.title,
    items: group.items.map(item => itemFor(contributionId, slot, item)),
  }));
}

function menusFor(registration: UiRegistration): readonly RibbonTab[] {
  return (registration.menu ?? []).map(menu => {
    const tabKey = key(registration.id, 'menu', menu.id);
    // One menu becomes one tab with one section: the package's menu shape has no
    // nested panels, so inventing several would be a second authoring model.
    const panel: RibbonPanel = {
      id: `${menu.id}.panel`,
      titleKey: `${tabKey}.panel`,
      title: menu.title,
      items: menu.items.map(item => itemFor(registration.id, 'menu', item)),
    };
    return { id: menu.id, labelKey: tabKey, label: menu.title, panels: [panel] };
  });
}

function trayFor(registration: UiRegistration): readonly TrayPanel[] {
  return (registration.tray ?? []).map(panel => ({
    id: panel.id,
    titleKey: key(registration.id, 'tray', panel.id),
    title: panel.title,
    kind: 'pending',
    pendingKey: `${key(registration.id, 'tray', panel.id)}.pending`,
    pendingText: panel.pendingReason,
  }));
}

function domainOf(registration: UiRegistration): DomainId | '*' {
  if (registration.domain === '*') return '*';
  if (!(DOMAIN_IDS as readonly string[]).includes(registration.domain)) {
    throw new CadApiError(
      'api.invalid_argument: ui.domain',
      `invalid argument: ui.domain (unknown host domain ${registration.domain})`,
    );
  }
  return registration.domain as DomainId;
}

/**
 * The host patch one contribution becomes.
 *
 * Pure: the caller registers it. A contribution with no slot is already refused
 * by the package, so this never returns an empty patch.
 */
export function uiPatchFor(registration: UiRegistration): DomainUiPatch {
  const menu = menusFor(registration);
  const toolbar = panelsFor(registration.id, 'toolbar', registration.toolbar);
  const tray = trayFor(registration);
  return {
    id: registration.id,
    domain: domainOf(registration),
    ...(menu.length === 0 ? {} : { menu }),
    ...(toolbar.length === 0 ? {} : { toolbar }),
    ...(tray.length === 0 ? {} : { tray }),
  };
}

/** Install one plugin contribution into the live domain registry. */
export function installPluginUi(registration: UiRegistration): void {
  registerPluginUi(uiPatchFor(registration));
}

/** Remove one plugin contribution; an unknown id is a no-op. */
export function removePluginUi(id: string): boolean {
  return unregisterPluginUi(id);
}
