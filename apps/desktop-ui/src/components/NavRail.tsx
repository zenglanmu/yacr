// Navigation rail: a slim vertical toolbar at the right edge of the CAD
// partition (DOM chrome inside the floating dock column — the dock is an opaque
// overlay that paints over the drawing area's right edge, never a transparent
// hit-through layer; see `NavigationDock.tsx`).
//
// The rail is collapsible. On the desktop its expanded state is the host's
// `NAVBAR` setting (`cad_status.navigation.navbarVisible`), so the chevron is a
// real `NAVBAR` command and the host remains the authority; when the setting is
// off only a slim chevron remains, titled with the catalog's own "the navigation
// bar is off" sentence. On the compact layout the rail starts collapsed even
// when the host's navbar is on, and the chevron expands it locally — the dock's
// width never changes, so the drawing area's rect does not move either.
import React, { useLayoutEffect, useState } from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { commandById } from '../commands/commands.ts';
import { palettesOffered } from '../cad/surface.ts';
import { navigationSettings } from '../cad/viewcube.ts';

const NAV_ITEMS = ['FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'];

export function NavRail() {
  const { t } = useI18n();
  const shell = useShell();
  // Compact rail state is local because the compact layout has no desktop navbar
  // setting to read: the rail starts closed and the chevron opens it. A live
  // breakpoint crossing returns it to the collapsed default; ordinary renders
  // (status, theme, a palette toggle) never close a rail the user opened.
  const mobile = shell.mobile;
  const [mobileOpen, setMobileOpen] = useState(false);
  useLayoutEffect(() => {
    if (mobile) setMobileOpen(false);
  }, [mobile]);
  const navigation = navigationSettings(shell.status?.navigation);
  const open = mobile ? mobileOpen : navigation.navbarVisible;
  const label = mobile
    ? (open ? t('navrail.collapse') : t('navrail.expand'))
    : (navigation.navbarVisible ? t('navrail.collapse') : t('navrail.hidden'));
  const toggle = mobile
    ? () => setMobileOpen(value => !value)
    : () => shell.runCommand('NAVBAR');
  // Palette toggles exist only where the desktop palettes do (a `dom-canvas`
  // carrier holds no host-side document state for them to read). One carrier
  // rule — `palettesOffered` — decides for **every** toggle below: the layers,
  // file-info and diagnostics toggles stand or fall together, so no button here
  // can toggle a panel that is never rendered. The command buttons above are
  // live on every carrier and are unaffected.
  const palettes = palettesOffered(shell);
  return (
    <div className="navrail" role="toolbar" aria-label={t('shell.nav')}>
      <button
        type="button"
        className="navrail__button navrail__toggle"
        aria-expanded={open}
        aria-label={label}
        title={label}
        onClick={toggle}
      >
        <Icon name={open ? 'chevronRight' : 'chevronLeft'} size={16} />
      </button>
      {open ? (
        <>
          {NAV_ITEMS.map(id => {
            const command = commandById(id)!;
            const enabled = shell.canCommand(id);
            return (
              <button
                key={id}
                type="button"
                className="navrail__button"
                disabled={!enabled}
                title={enabled ? t(command.labelKey) : t('command.unsupported', { command: t(command.labelKey) })}
                aria-label={t(command.labelKey)}
                onClick={() => shell.runCommand(id)}
              >
                <Icon name={command.icon as IconName} size={18} />
              </button>
            );
          })}
          {palettes ? (
            <>
              <div className="navrail__rule" aria-hidden="true" />
              <button
                type="button"
                className={['navrail__button', shell.layersOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
                aria-pressed={Boolean(shell.layersOpen)}
                title={t('layers.panel')}
                aria-label={t('layers.panel')}
                onClick={() => shell.toggleLayers()}
              >
                <Icon name="layers" size={18} />
              </button>
              <button
                type="button"
                className={['navrail__button', shell.fileInfoOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
                aria-pressed={Boolean(shell.fileInfoOpen)}
                title={t('fileinfo.title')}
                aria-label={t('fileinfo.title')}
                onClick={() => shell.setFileInfoOpen(!shell.fileInfoOpen)}
              >
                <Icon name="info" size={18} />
              </button>
              <button
                type="button"
                className={['navrail__button', shell.diagnosticsOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
                aria-pressed={Boolean(shell.diagnosticsOpen)}
                title={t('diagnostics.title')}
                aria-label={t('diagnostics.title')}
                onClick={() => shell.setDiagnosticsOpen(!shell.diagnosticsOpen)}
              >
                <Icon name="diagnostics" size={18} />
              </button>
              <button
                type="button"
                className={['navrail__button', shell.referencesOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
                aria-pressed={Boolean(shell.referencesOpen)}
                title={t('resources.references')}
                aria-label={t('resources.references')}
                onClick={() => shell.setReferencesOpen(!shell.referencesOpen)}
              >
                <Icon name="reference" size={18} />
              </button>
            </>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
