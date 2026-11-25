// The plugin `Ui` namespace: declarative menu/toolbar/tray contributions.
//
// Registration is **client-local**, exactly like `Runtime.registerCommand`: a
// plugin hands over pure data, this module validates it, and the host hook
// `createCadApi` was given (`onUiRegister`) turns it into whatever its chrome
// renders. No wire request is involved, so the capability descriptor has no
// `Ui.register` row: advertising one would promise a host round-trip that never
// happens.
//
// The shape is deliberately **not** React and not the host's `RibbonTab` types.
// A plugin names menus, groups, items and tray panels; the host maps them onto
// its own slot model. A plugin can therefore extend the interface without
// importing host source, and the one dispatch funnel (`cad_command`) still owns
// every command an item can run.
//
// Labels are **plugin-supplied literal text** (`label` / `title` /
// `pendingReason`), not catalog keys: a third-party plugin cannot add rows to
// the host's i18n catalogs. The text is rendered verbatim; a plugin that wants
// locale switching localizes before registering (v1 limitation, documented).
//
// There is no sandbox here: the plugin is a same-privilege module (ADR
// 0029/0030/0034), and this namespace only limits what the host is asked to
// render, not what a malicious plugin can do.

import { CadApiError, invalidArgument } from './errors.ts';

/** One clickable item in a plugin menu or toolbar group. */
export interface UiItem {
  /** Stable id within its group (`[A-Za-z0-9][A-Za-z0-9_.:-]*`). */
  readonly id: string;
  /** Literal display text, rendered verbatim (never a catalog key). */
  readonly label: string;
  /**
   * A host glyph name. An unknown name renders the host's `dot` fallback; the
   * package does not know the host's icon set, so it only checks the string.
   */
  readonly icon?: string;
  /**
   * The command the item runs through the shared funnel. Exactly one of
   * `command` / `pendingReason` is present.
   */
  readonly command?: string;
  /**
   * Why the item cannot run in this build (literal sentence). Exactly one of
   * `command` / `pendingReason` is present; a pending row is labelled and
   * disabled, never a fake button.
   */
  readonly pendingReason?: string;
}

/** One top-level menu or one toolbar group. */
export interface UiMenu {
  /** Stable id (`[A-Za-z0-9][A-Za-z0-9_.:-]*`), unique inside the contribution. */
  readonly id: string;
  /** Literal title, rendered verbatim. */
  readonly title: string;
  /** At least one item; an empty group would be an empty titled box. */
  readonly items: readonly UiItem[];
}

/** One toolbar group (the same shape as a menu). */
export type UiGroup = UiMenu;

/** One tray panel. Only the honest `pending` body is representable today. */
export interface UiTrayPanel {
  /** Stable id, unique inside the contribution. */
  readonly id: string;
  /** Literal title, rendered verbatim. */
  readonly title: string;
  /** Why the panel has no real body yet (literal sentence). */
  readonly pendingReason: string;
}

/**
 * One declarative contribution.
 *
 * `domain` targets a single application domain or `'*'` (the default) for every
 * profile. At least one slot must be present. `id` is the unregistration handle.
 */
export interface UiContribution {
  readonly id: string;
  readonly domain?: string;
  readonly menu?: readonly UiMenu[];
  readonly toolbar?: readonly UiGroup[];
  readonly tray?: readonly UiTrayPanel[];
}

/** The frozen, validated contribution the host hook receives. */
export interface UiRegistration extends UiContribution {
  readonly domain: string;
  readonly menu?: readonly UiMenu[];
  readonly toolbar?: readonly UiGroup[];
  readonly tray?: readonly UiTrayPanel[];
}

/** The host hooks a UI namespace reports to. Both are optional. */
export interface UiHooks {
  readonly onUiRegister?: (registration: UiRegistration) => void;
  readonly onUiUnregister?: (id: string) => void;
}

/** The `Ui` namespace exposed on `CadApi.ui`. */
export interface UiNamespace {
  /**
   * Register one contribution.
   *
   * Refuses a malformed shape (`api.invalid_argument: ui.*`), a duplicate `id`
   * in this client (`api.invalid_argument: ui.duplicate`) and — when the host
   * supplied no hook — the whole operation
   * (`unsupported-runtime: cad_api.Ui.register`).
   */
  register(contribution: UiContribution): UiRegistration;
  /**
   * Remove one contribution by its `id`.
   *
   * An unknown id is `api.invalid_argument: ui.unknown`; a malformed id is
   * `api.invalid_argument: ui.id`.
   */
  unregister(id: string): void;
}

/** `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}` — contribution, group and item ids. */
const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/;

/** A host domain token (`'*'` is handled separately). */
const DOMAIN_PATTERN = /^[a-z0-9][a-z0-9_.-]*$/;

/**
 * A host command token.
 *
 * Host commands may contain spaces (`ZOOM WINDOW`, `TCASE T`), so the grammar is
 * printable ASCII rather than the narrower `Runtime` command-name pattern. The
 * host's own admission still decides whether the command can run; this only
 * refuses text that could not be a command at all.
 */
const COMMAND_PATTERN = /^[\x20-\x7e]+$/;

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function nonEmpty(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

function readId(value: unknown, field: string): string {
  if (typeof value !== 'string' || !ID_PATTERN.test(value)) throw invalidArgument(field);
  return value;
}

function readText(value: unknown, field: string): string {
  if (!nonEmpty(value)) throw invalidArgument(field);
  return value;
}

function readItems(value: unknown, field: string): readonly UiItem[] {
  if (!Array.isArray(value) || value.length === 0) throw invalidArgument(`${field}.items`);
  const seen = new Set<string>();
  const items = value.map((entry, index) => {
    if (!isRecord(entry)) throw invalidArgument(`${field}.items[${index}]`);
    const id = readId(entry['id'], `${field}.items[${index}].id`);
    if (seen.has(id)) throw invalidArgument(`${field}.items[${index}].id.duplicate`);
    seen.add(id);
    const label = readText(entry['label'], `${field}.items[${index}].label`);
    const icon = entry['icon'];
    if (icon !== undefined && !nonEmpty(icon)) throw invalidArgument(`${field}.items[${index}].icon`);
    const command = entry['command'];
    const pendingReason = entry['pendingReason'];
    if ((command === undefined) === (pendingReason === undefined)) {
      throw invalidArgument(`${field}.items[${index}].dispatch`);
    }
    let readonlyCommand: string | undefined;
    if (command !== undefined) {
      if (typeof command !== 'string' || !COMMAND_PATTERN.test(command) || command !== command.trim()) {
        throw invalidArgument(`${field}.items[${index}].command`);
      }
      readonlyCommand = command;
    }
    let readonlyReason: string | undefined;
    if (pendingReason !== undefined) {
      readonlyReason = readText(pendingReason, `${field}.items[${index}].pendingReason`);
    }
    return Object.freeze({
      id,
      label,
      ...(icon === undefined ? {} : { icon: icon as string }),
      ...(readonlyCommand === undefined ? {} : { command: readonlyCommand }),
      ...(readonlyReason === undefined ? {} : { pendingReason: readonlyReason }),
    });
  });
  return Object.freeze(items);
}

function readGroups(value: unknown, field: string): readonly UiGroup[] {
  if (!Array.isArray(value) || value.length === 0) throw invalidArgument(field);
  const seen = new Set<string>();
  const groups = value.map((entry, index) => {
    if (!isRecord(entry)) throw invalidArgument(`${field}[${index}]`);
    const id = readId(entry['id'], `${field}[${index}].id`);
    if (seen.has(id)) throw invalidArgument(`${field}[${index}].id.duplicate`);
    seen.add(id);
    const title = readText(entry['title'], `${field}[${index}].title`);
    const items = readItems(entry['items'], `${field}[${index}]`);
    return Object.freeze({ id, title, items });
  });
  return Object.freeze(groups);
}

function readTray(value: unknown): readonly UiTrayPanel[] {
  if (!Array.isArray(value) || value.length === 0) throw invalidArgument('tray');
  const seen = new Set<string>();
  const panels = value.map((entry, index) => {
    if (!isRecord(entry)) throw invalidArgument(`tray[${index}]`);
    const id = readId(entry['id'], `tray[${index}].id`);
    if (seen.has(id)) throw invalidArgument(`tray[${index}].id.duplicate`);
    seen.add(id);
    const title = readText(entry['title'], `tray[${index}].title`);
    const pendingReason = readText(entry['pendingReason'], `tray[${index}].pendingReason`);
    return Object.freeze({ id, title, pendingReason });
  });
  return Object.freeze(panels);
}

function readDomain(value: unknown): string {
  if (value === undefined) return '*';
  if (value === '*') return '*';
  if (typeof value !== 'string' || !DOMAIN_PATTERN.test(value)) throw invalidArgument('domain');
  return value;
}

/**
 * Validate one contribution and return the frozen registration.
 *
 * Pure: no hook is called here, so the caller can reject a shape without
 * touching the host. Every refusal names the offending field.
 */
function readContribution(value: unknown): UiRegistration {
  if (!isRecord(value)) throw invalidArgument('ui');
  const id = readId(value['id'], 'id');
  const domain = readDomain(value['domain']);
  const menu = value['menu'] === undefined ? undefined : readGroups(value['menu'], 'menu');
  const toolbar = value['toolbar'] === undefined ? undefined : readGroups(value['toolbar'], 'toolbar');
  const tray = value['tray'] === undefined ? undefined : readTray(value['tray']);
  if (menu === undefined && toolbar === undefined && tray === undefined) {
    throw invalidArgument('ui.slots');
  }
  return Object.freeze({
    id,
    domain,
    ...(menu === undefined ? {} : { menu }),
    ...(toolbar === undefined ? {} : { toolbar }),
    ...(tray === undefined ? {} : { tray }),
  });
}

/**
 * Build the UI namespace for one client.
 *
 * Registrations live for the lifetime of this namespace (one `createCadApi`
 * call). The loader unregisters them on deactivate; `ui.unregister` is the
 * explicit path for a plugin that wants to remove one earlier.
 */
export function createUiNamespace(hooks: UiHooks): UiNamespace {
  const registrations = new Map<string, UiRegistration>();

  return {
    register(contribution: UiContribution): UiRegistration {
      const registration = readContribution(contribution);
      if (hooks.onUiRegister === undefined) {
        // Accepting the contribution would promise a chrome that never renders it.
        throw new CadApiError(
          'unsupported-runtime: cad_api.Ui.register',
          'this host has no plugin UI surface',
        );
      }
      if (registrations.has(registration.id)) throw invalidArgument('ui.duplicate');
      hooks.onUiRegister(registration);
      registrations.set(registration.id, registration);
      return registration;
    },
    unregister(id): void {
      if (typeof id !== 'string' || !ID_PATTERN.test(id)) throw invalidArgument('ui.id');
      if (!registrations.has(id)) throw invalidArgument('ui.unknown');
      registrations.delete(id);
      hooks.onUiUnregister?.(id);
    },
  };
}
