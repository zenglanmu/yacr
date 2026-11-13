// Navigation rail: the icon column under the dock's head.
//
// Presentational only. Whether the rail is open is owned by
// `layout/nav-rail-state.ts` (host `NAVBAR` truth on the desktop, local state on
// the compact layout) and the one collapse/expand control lives in the dock's
// head, directly under the ViewCube — so this component takes `open` as a prop
// and renders nothing at all when it is closed. That is deliberate: a closed
// rail is not an empty toolbar, it is absent, which is what lets the compact dock
// shrink to the cube plus one icon instead of showing a full-height empty strip.
//
// Inside the rail: the five view commands, then the palette toggles behind the
// one carrier gate (`palettesOffered`) — the layers, file-info, diagnostics and
// references toggles stand or fall together, so no button here can toggle a panel
// that is never rendered. Command ids and gates are unchanged from the rail this
// replaces; only the chevron moved out.
import React from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { commandById } from '../commands/commands.ts';
import { palettesOffered } from '../cad/surface.ts';

const NAV_ITEMS = ['FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'];

/** The rail is a pure projection of the dock's open state. */
export interface NavRailProps {
  /** Whether the rail's items are shown; `false` renders nothing. */
  readonly open: boolean;
}

export function NavRail({ open }: NavRailProps) {
  const { t } = useI18n();
  const shell = useShell();
  if (!open) return null;
  const palettes = palettesOffered(shell);
  return (
    <div className="navrail" role="toolbar" aria-label={t('shell.nav')}>
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
    </div>
  );
}
