// Navigation rail: a slim vertical toolbar docked at the right edge of the CAD
// partition (DOM chrome at the canvas edge — never a floating overlay across the
// drawing area, which the host's native compositing does not support).
import React from 'react';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { commandById } from '../commands.ts';
import { palettesOffered } from '../surface.ts';

const NAV_ITEMS = ['FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'];

export function NavRail() {
  const { t } = useI18n();
  const shell = useShell();
  // Palette toggles exist only where the docked palettes do (a `dom-canvas`
  // carrier holds no host-side document state for them to read). One carrier
  // rule — `palettesOffered` — decides for **every** toggle below: the layers
  // pair and the diagnostics pair stand or fall together, so no button here can
  // toggle a panel that is never rendered. The command buttons above are live on
  // every carrier and are unaffected.
  const palettes = palettesOffered(shell);
  return (
    <div className="region region--navrail navrail app-region" role="toolbar" aria-label={t('shell.nav')}>
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
            className={['navrail__button', shell.diagnosticsOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
            aria-pressed={Boolean(shell.diagnosticsOpen)}
            title={t('diagnostics.title')}
            aria-label={t('diagnostics.title')}
            onClick={() => shell.setDiagnosticsOpen(!shell.diagnosticsOpen)}
          >
            <Icon name="diagnostics" size={18} />
          </button>
        </>
      ) : null}
    </div>
  );
}
