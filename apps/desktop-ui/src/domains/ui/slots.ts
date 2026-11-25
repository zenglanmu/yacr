// Domain UI slots: the one presentation contract every domain profile and plugin
// contributes to.
//
// The React tree has one shape, but the chrome above the drawing is assembled
// from **named slots** rather than hard-wired regions:
//
//   * `ribbon`    the top ribbon band (`bim`'s Revit-style tabs and panels);
//   * `menu`      the top menu bar (the drafting profile's ribbon collapses to
//                 the same renderer; `concept` uses it as a real menubar);
//   * `toolbar`   the left tool column (`concept`'s Large Tool Set);
//   * `tray`      the right default tray (`concept`'s Entity Info / Tags / ...).
//
// A contribution is **data**, never JSX: the same `RibbonTab`/`RibbonPanel`/
// `RibbonItem` vocabulary the drafting ribbon already uses, plus a small tray
// panel descriptor. That is what lets a plugin extend the interface without
// shipping React, and what keeps dispatch on the one funnel (`runItem` →
// `cad_command` / a shell action) instead of opening a second path.
//
// This module is React-free and side-effect free; `registry.ts` owns the mutable
// tables and `use-ui-layout.ts` is the React binding.
import type { RibbonPanel, RibbonTab } from '../../ribbon/ribbon-model.ts';
import type { DomainId } from '../domains.ts';

/** The slots a domain (or plugin) can fill today. */
export type DomainSlot = 'ribbon' | 'menu' | 'toolbar' | 'tray';

/**
 * A default-tray panel.
 *
 * `kind` names the **built-in** body the tray renders. A plugin cannot inject
 * arbitrary React; it can add a declarative panel (`pending` with its own
 * sentence) or one of the registered bodies. Adding a new body is a code change
 * in `DomainTray.tsx`, which keeps the tray's write paths reviewable.
 */
export type TrayPanelKind =
  | 'entityInfo'
  | 'components'
  | 'styles'
  | 'tags'
  | 'scenes'
  | 'instructor'
  | 'bimQuery'
  | 'pending';

/** One tray panel, in vertical order. */
export interface TrayPanel {
  readonly id: string;
  readonly titleKey: string;
  /** Literal title from a plugin contribution; wins over `titleKey`. */
  readonly title?: string;
  readonly kind: TrayPanelKind;
  /** For `kind: 'pending'`: the stable catalog sentence naming what is missing. */
  readonly pendingKey?: string;
  /** A plugin-supplied literal version of that sentence. */
  readonly pendingText?: string;
}

/** One domain's UI contribution. Every slot is optional. */
export interface DomainUiLayout {
  /**
   * Top ribbon tabs (`RibbonTab` = one tab + its panels). Rendered as a real
   * band by `DomainRibbonBand`; this is the Revit-style surface, distinct from
   * `menu` (a dropdown menubar).
   */
  readonly ribbon?: readonly RibbonTab[];
  /** Top-level menus (`RibbonTab` = one trigger + its sections). */
  readonly menu?: readonly RibbonTab[];
  /** Left tool groups (`RibbonPanel` = one titled group of items). */
  readonly toolbar?: readonly RibbonPanel[];
  /** Right default-tray panels. */
  readonly tray?: readonly TrayPanel[];
}

/**
 * A runtime patch from a plugin.
 *
 * `domain: '*'` applies to every profile. Patches merge **additively**: a menu
 * or toolbar group with an existing id keeps its order and gains the patch's
 * panels/items; a new id is appended. A tray panel with the same id replaces
 * the domain's panel (the later registration wins), so a plugin can restyle a
 * panel without duplicating it.
 */
export interface DomainUiPatch extends DomainUiLayout {
  readonly id: string;
  readonly domain: DomainId | '*';
}
