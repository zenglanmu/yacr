// The Tray Settings surface (`TRAYSETTINGS`): the status bar's tray switch.
//
// The surface owns **no state**. Its one checkbox is the session
// `TRAYSETTINGS` flag, read from `cad_status.traySettings` (the same value the
// status bar uses to decide whether its tray is painted) and written by
// dispatching the router command through the normal command funnel. A host that
// has no status bar at all never reaches this component: the typed word is
// refused by name in `commands/local-selectors.ts` before dispatch.
//
// The contents line names what this build's tray really holds — the status-bar
// utility/notification cluster — instead of reproducing an AutoCAD tray icon
// list this build does not have.
import React from 'react';
import { Icon } from '../icons.tsx';
import { IconButton, Toggle } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { traySettingsOn, TRAY_SETTINGS_COMMAND } from '../cad/cad.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';
import { closeTraySettings } from '../chrome/surfaces.ts';

export function TraySettingsSurface() {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(closeTraySettings);
  const on = traySettingsOn(shell.status);
  const admitted = shell.canCommand(TRAY_SETTINGS_COMMAND);

  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="workspace-panel tray-panel"
      role="dialog"
      aria-label={t('tray.title')}
    >
      <header className="workspace-panel__head">
        <span className="workspace-panel__title">
          <Icon name="settings" size={14} />
          {t('tray.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('tray.close')} onClick={closeTraySettings} />
      </header>
      <p className="workspace-panel__hint">{t('tray.hint')}</p>
      <div className="tray-panel__row">
        <Toggle
          label={t('tray.toggle')}
          on={on}
          disabled={!admitted}
          title={t(on ? 'tray.toggle_hint_on' : 'tray.toggle_hint_off')}
          onClick={() => shell.runCommand(TRAY_SETTINGS_COMMAND)}
        />
      </div>
      <p className="tray-panel__contents">{t('tray.contents')}</p>
    </aside>
  );
}
