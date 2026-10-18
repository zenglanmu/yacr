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
import React, { useEffect, useState } from 'react';
import { Icon } from '../icons.jsx';
import { IconButton, RibbonButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { RIBBON_TABS, ribbonTab } from '../ribbon-model.mjs';

function runItem(shell, item) {
  if (item.action) return shell.runAction(item.action);
  if (item.command) return shell.runCommand(item.command);
  return undefined;
}

function unavailable(t, item) {
  return t('ribbon.command_unsupported', { command: t(item.labelKey) });
}

function ItemButton({ item }) {
  const { t } = useI18n();
  const shell = useShell();
  const enabled = item.action ? true : item.command ? shell.canCommand(item.command) : false;
  if (!enabled) {
    return <RibbonButton icon={item.icon} label={t(item.labelKey)} disabled hint={unavailable(t, item)} />;
  }
  return <RibbonButton icon={item.icon} label={t(item.labelKey)} onClick={() => runItem(shell, item)} />;
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
        const enabled = variant.action ? true : variant.command ? shell.canCommand(variant.command) : false;
        return (
          <button
            key={variant.id}
            type="button"
            role="menuitem"
            className={['ribbon-menu-item', enabled ? '' : 'is-disabled'].filter(Boolean).join(' ')}
            disabled={!enabled}
            title={enabled ? undefined : unavailable(t, variant)}
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
          <div key={`${panel.id}-${index}`} className={group[0].large ? 'ribbon-panel__group is-large' : 'ribbon-panel__group is-small'}>
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

export function Ribbon() {
  const { t } = useI18n();
  const shell = useShell();
  const [openMenu, setOpenMenu] = useState(null);
  const tab = ribbonTab(shell.ribbonTab);
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
        <div className="ribbon__panels" role="tabpanel" aria-label={t(tab.labelKey)}>
          {tab.panels.map(panel => (
            <Panel key={panel.id} panel={panel} openMenu={openMenu} setOpenMenu={setOpenMenu} />
          ))}
        </div>
      )}
    </div>
  );
}
