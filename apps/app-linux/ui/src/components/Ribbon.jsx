// Ribbon: tab strip (File / Home / Draw / Measure / Annotate / View) with panels
// inside each tab. Panel titles sit at the bottom, matching the desktop ribbon
// convention, and a dialog-launcher arrow marks panels with deeper settings.
//
// The panel band has one fixed height (--ribbon-h, see style.css) on every tab,
// so switching tabs never reflows the chrome nor the CAD partition rect.
//
// Variant drop-downs open as an *in-band popover*: the menu is absolutely
// positioned inside the panel body, so it stays inside the fixed band and never
// paints below it over the native CAD partition. It scrolls internally if the
// list is taller than the body. Escape or a click outside dismisses it.
//
// The panel strip is a horizontal scroller (`overflow-x`), because a 92px band
// cannot hold every tool of the widest tab at a 960px window. A strip that runs
// past the right edge is exactly where a reader loses tools without noticing,
// so each side that actually hides content gets an edge fade plus a chevron that
// pages the strip (see `ribbon-overflow.mjs` and `.ribbon__overflow`). The cues
// are overlays: they never take width from the panels, so the band measures the
// same whether or not they are shown.
import React, { useEffect, useRef, useState } from 'react';
import { Icon } from '../icons.jsx';
import { IconButton, RibbonButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { RIBBON_TABS, ribbonTab } from '../ribbon-model.mjs';
import { DOCUMENT_SCOPED_COMMANDS, NO_DOCUMENT_KEY, unavailableReasonKey } from '../commands.mjs';
import { standardViewAvailable, standardViewUnavailableReasonKey } from '../view3d.mjs';
import { ribbonOverflow, ribbonScrollTarget, sameOverflow } from '../ribbon-overflow.mjs';

/** No strip, no overflow: also the answer before the first measurement. */
const NO_OVERFLOW = Object.freeze({ overflowing: false, start: false, end: false });

function runItem(shell, item) {
  if (item.action) return shell.runAction(item.action);
  // A standard view carries a payload, so it never rides the name-only command
  // funnel: the shell sends it on `cad_standard_view` through its own typed
  // helper. Order matters — the typed branch is checked before `command`.
  if (item.standardView) return shell.runStandardView(item.standardView);
  if (item.command) return shell.runCommand(item.command);
  return undefined;
}

/**
 * Why `item` cannot run here, in catalog words.
 *
 * Three explicit reasons, never a vague "unsupported": the orbit gesture hint
 * for a drag-only command, the catalog's own "no drawing is open" sentence for a
 * document-scoped selector or a standard view on the start page, and the generic
 * unsupported reason for a command the router does not admit.
 */
function unavailable(t, item, shell) {
  if (item.standardView) {
    return t(standardViewUnavailableReasonKey(shell), { command: t(item.labelKey) });
  }
  if (item.command && DOCUMENT_SCOPED_COMMANDS.includes(item.command)
    && shell.status && !shell.loading && !shell.documentOpen) {
    return t(NO_DOCUMENT_KEY, { command: t(item.labelKey) });
  }
  return t(unavailableReasonKey(item.command), { command: t(item.labelKey) });
}

function ItemButton({ item }) {
  const { t } = useI18n();
  const shell = useShell();
  // Three gates, one per dispatch token: a shell action is always available, a
  // standard view needs a loaded status with an open drawing, and a command is
  // enabled only by router admission. Nothing is ever locally enabled.
  const enabled = item.action ? true
    : item.standardView ? standardViewAvailable(shell)
      : item.command ? shell.canCommand(item.command)
        : false;
  // The About dialog is a host window, so `cad_status.popup` is the only honest
  // "open" signal. The button reads pressed while it is open and stays
  // clickable: the host answers `existing: true` and focuses the live window.
  const active = item.action === 'about' ? Boolean(shell.popupOpen) : false;
  if (!enabled) {
    return <RibbonButton icon={item.icon} label={t(item.labelKey)} disabled hint={unavailable(t, item, shell)} />;
  }
  return <RibbonButton icon={item.icon} label={t(item.labelKey)} active={active} onClick={() => runItem(shell, item)} />;
}

function PanelItem({ item, openMenu, setOpenMenu }) {
  const { t } = useI18n();
  const hasVariants = Boolean(item.variants?.length);
  const menuOpen = openMenu === item.id;
  return (
    <div className={['ribbon-panel__item', item.large ? 'is-large' : 'is-small', hasVariants ? 'has-variants' : '', menuOpen ? 'is-open' : ''].filter(Boolean).join(' ')}>
      <ItemButton item={item} />
      {hasVariants ? (
        <button
          type="button"
          className="ribbon-item__caret"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          aria-label={t('ribbon.variants', { name: t(item.labelKey) })}
          title={t('ribbon.variants', { name: t(item.labelKey) })}
          onClick={() => setOpenMenu(menuOpen ? null : item.id)}
        >
          <Icon name="chevronDown" size={12} />
        </button>
      ) : null}
    </div>
  );
}

function VariantMenu({ item, onClose }) {
  const { t } = useI18n();
  const shell = useShell();
  return (
    <div className="ribbon-panel__menu" role="menu" aria-label={t('ribbon.variants', { name: t(item.labelKey) })}>
      {(item.variants ?? []).map(variant => {
        const enabled = variant.action ? true : variant.standardView ? standardViewAvailable(shell) : variant.command ? shell.canCommand(variant.command) : false;
        return (
          <button
            key={variant.id}
            type="button"
            role="menuitem"
            className={['ribbon-menu-item', enabled ? '' : 'is-disabled'].filter(Boolean).join(' ')}
            disabled={!enabled}
            title={enabled ? undefined : unavailable(t, variant, shell)}
            onClick={() => { runItem(shell, variant); onClose(); }}
          >
            <Icon name={variant.icon ?? item.icon} size={15} />
            <span>{t(variant.labelKey)}</span>
            {enabled ? null : <em className="ribbon-menu-item__badge">{t('command.badge.unwired')}</em>}
          </button>
        );
      })}
    </div>
  );
}

function Panel({ panel, openMenu, setOpenMenu }) {
  const { t } = useI18n();
  // Group consecutive small items into one cluster; small clusters render as a
  // column-major grid that wraps across the fixed band height.
  const groups = [];
  for (const item of panel.items) {
    if (item.large) groups.push([item]);
    else if (groups.length && !groups.at(-1).some(entry => entry.large)) groups.at(-1).push(item);
    else groups.push([item]);
  }
  const openItem = panel.items.find(item => item.id === openMenu && item.variants?.length);
  return (
    <section className="ribbon-panel" aria-label={t(panel.titleKey)}>
      <div className="ribbon-panel__body">
        {groups.map((group, index) => (
          <div
            key={`${panel.id}-${index}`}
            className={[
              group[0].large ? 'ribbon-panel__group is-large' : 'ribbon-panel__group is-small',
              !group[0].large && panel.rows === 2 ? 'is-rows-2' : '',
            ].filter(Boolean).join(' ')}
          >
            {group.map(item => <PanelItem key={item.id} item={item} openMenu={openMenu} setOpenMenu={setOpenMenu} />)}
          </div>
        ))}
        {openItem ? <VariantMenu item={openItem} onClose={() => setOpenMenu(null)} /> : null}
      </div>
      <footer className="ribbon-panel__title">
        <span>{t(panel.titleKey)}</span>
        {panel.launcher ? (
          <IconButton
            icon="launcher"
            size="xs"
            label={t('ribbon.panel_settings')}
            disabled
            title={t('ribbon.command_unsupported', { command: t('ribbon.panel_settings') })}
          />
        ) : null}
      </footer>
    </section>
  );
}

/**
 * Live overflow state of the panel strip.
 *
 * The first answer is deliberately "nothing overflows" (no cue): a cue is only
 * honest once the strip has been measured. `resetKey` is whatever makes the
 * strip render different content — a tab switch or a locale change — and also
 * re-homes the scroll offset, because a new tab must start at its left edge
 * instead of inheriting the previous tab's offset.
 *
 * A ResizeObserver catches window resizes; the scroll listener catches the
 * reader paging the strip; one `requestAnimationFrame` re-measure catches the
 * late layout (web fonts, panel titles) that a synchronous read misses.
 */
function useRibbonOverflow(panelsRef, resetKey) {
  const [state, setState] = useState(NO_OVERFLOW);
  useEffect(() => {
    const node = panelsRef.current;
    if (!node) {
      setState(NO_OVERFLOW);
      return undefined;
    }
    node.scrollLeft = 0;
    const measure = () => setState(previous => {
      const next = ribbonOverflow(node);
      return sameOverflow(previous, next) ? previous : next;
    });
    measure();
    const frame = typeof requestAnimationFrame === 'function' ? requestAnimationFrame(measure) : 0;
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    node.addEventListener('scroll', measure, { passive: true });
    window.addEventListener('resize', measure);
    observer?.observe(node);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      node.removeEventListener('scroll', measure);
      window.removeEventListener('resize', measure);
      observer?.disconnect();
    };
  }, [panelsRef, resetKey]);
  return state;
}

/** Page the panel strip one screen-ish step toward `direction`. */
function pageRibbon(node, direction) {
  if (!node) return;
  const left = ribbonScrollTarget(node, direction);
  // Smooth paging keeps the reader oriented; the assignment is the plain
  // fallback for an engine without scroll options.
  if (typeof node.scrollTo === 'function') node.scrollTo({ left, behavior: 'smooth' });
  else node.scrollLeft = left;
}

/**
 * One edge cue: an edge fade plus a chevron that pages the strip.
 *
 * Rendered only for a side that actually hides content, so a maximised window
 * shows none. It is a real button (focusable, labelled) rather than a decorative
 * gradient, because the reader must be able to *reach* the hidden panels, not
 * just learn that they exist.
 */
function OverflowCue({ side, onPage }) {
  const { t } = useI18n();
  const label = t(side === 'start' ? 'ribbon.scroll_left' : 'ribbon.scroll_right');
  return (
    <button
      type="button"
      className={`ribbon__overflow is-${side}`}
      aria-label={label}
      title={label}
      onClick={onPage}
    >
      <Icon name={side === 'start' ? 'chevronLeft' : 'chevronRight'} size={16} />
    </button>
  );
}

export function Ribbon() {
  const { t, locale } = useI18n();
  const shell = useShell();
  const [openMenu, setOpenMenu] = useState(null);
  const panelsRef = useRef(null);
  const tab = ribbonTab(shell.ribbonTab);
  // A tab switch or a language switch renders different panel widths, so both
  // re-measure (and re-home) the strip.
  const overflow = useRibbonOverflow(panelsRef, `${shell.ribbonTab}/${locale}`);
  // Dismiss an open variant popover on Escape or a click outside it. The
  // invoking item sits above the menu, so its caret still toggles it closed.
  useEffect(() => {
    if (!openMenu) return undefined;
    const onKeyDown = event => { if (event.key === 'Escape') setOpenMenu(null); };
    const onPointerDown = event => { if (!event.target?.closest?.('.ribbon-panel__menu')) setOpenMenu(null); };
    document.addEventListener('keydown', onKeyDown);
    document.addEventListener('pointerdown', onPointerDown);
    return () => {
      document.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('pointerdown', onPointerDown);
    };
  }, [openMenu]);
  return (
    <div className={['region', 'region--ribbon', 'ribbon', 'app-region', shell.ribbonCollapsed ? 'is-collapsed' : ''].filter(Boolean).join(' ')}>
      <div className="ribbon__tabs" role="tablist" aria-label={t('ribbon.tabs_label')}>
        {RIBBON_TABS.map(entry => (
          <button
            key={entry.id}
            type="button"
            role="tab"
            aria-selected={entry.id === shell.ribbonTab}
            className={['ribbon-tab', entry.accent ? 'is-accent' : '', entry.id === shell.ribbonTab ? 'is-active' : ''].filter(Boolean).join(' ')}
            onClick={() => { setOpenMenu(null); shell.setRibbonTab(entry.id); }}
          >
            {t(entry.labelKey)}
          </button>
        ))}
        <div className="ribbon__tabs-spacer" />
        <IconButton
          icon={shell.ribbonCollapsed ? 'chevronDown' : 'chevronUp'}
          size="sm"
          label={shell.ribbonCollapsed ? t('ribbon.expand') : t('ribbon.collapse')}
          pressed={shell.ribbonCollapsed}
          onClick={() => { setOpenMenu(null); shell.setRibbonCollapsed(!shell.ribbonCollapsed); }}
        />
      </div>
      {shell.ribbonCollapsed ? null : (
        <div className="ribbon__band">
          <div className="ribbon__panels" role="tabpanel" aria-label={t(tab.labelKey)} ref={panelsRef}>
            {tab.panels.map(panel => (
              <Panel key={panel.id} panel={panel} openMenu={openMenu} setOpenMenu={setOpenMenu} />
            ))}
          </div>
          {overflow.start ? <OverflowCue side="start" onPage={() => pageRibbon(panelsRef.current, 'start')} /> : null}
          {overflow.end ? <OverflowCue side="end" onPage={() => pageRibbon(panelsRef.current, 'end')} /> : null}
        </div>
      )}
    </div>
  );
}
