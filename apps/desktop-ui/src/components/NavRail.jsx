// Navigation rail: a slim vertical toolbar docked at the right edge of the CAD
// partition (DOM chrome at the canvas edge — never a floating overlay across the
// drawing area, which the host's native compositing does not support).
import React from 'react';
import { Icon } from '../icons.jsx';
import { useI18n, useShell } from '../context.mjs';
import { commandById } from '../commands.mjs';

const NAV_ITEMS = ['FIT', 'ZOOMIN', 'ZOOMOUT', 'PAN', 'ORBIT'];

export function NavRail() {
  const { t } = useI18n();
  const shell = useShell();
  return (
    <div className="region region--navrail navrail app-region" role="toolbar" aria-label={t('shell.nav')}>
      {NAV_ITEMS.map(id => {
        const command = commandById(id);
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
            <Icon name={command.icon} size={18} />
          </button>
        );
      })}
      <div className="navrail__rule" aria-hidden="true" />
      {/* Palette toggles. The layers list exists only where the docked palettes
          do (a `dom-canvas` carrier holds no host-side layer table of its own),
          so the button is dropped there instead of toggling a panel that is never
          rendered. (The diagnostics toggle below predates this carrier flag and is
          still offered unconditionally — an existing gap, not this button's.) */}
      {shell.palettesAvailable !== false ? (
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
      ) : null}
      <button
        type="button"
        className={['navrail__button', shell.diagnosticsOpen ? 'is-on' : ''].filter(Boolean).join(' ')}
        aria-pressed={shell.diagnosticsOpen}
        title={t('diagnostics.title')}
        aria-label={t('diagnostics.title')}
        onClick={() => shell.setDiagnosticsOpen(!shell.diagnosticsOpen)}
      >
        <Icon name="diagnostics" size={18} />
      </button>
    </div>
  );
}
