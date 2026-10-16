// Diagnostics palette: docked (resizes the drawing area) rather than floating,
// because the host composites a native CAD surface in the central partition.
// Every value is either real host data or an explicit blank.
import React from 'react';
import { Icon } from '../icons.jsx';
import { Button, IconButton } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';

function value(input) {
  if (input === null || input === undefined) return '\u2014';
  if (typeof input === 'object') return JSON.stringify(input);
  if (typeof input === 'boolean') return input ? 'true' : 'false';
  return String(input);
}

function Row({ label, children }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

export function DiagnosticsPalette() {
  const { t } = useI18n();
  const shell = useShell();
  const status = shell.status;
  return (
    <section className="palette diagnostics" aria-label={t('diagnostics.title')}>
      <header className="palette__head">
        <h2><Icon name="diagnostics" size={15} />{t('diagnostics.title')}</h2>
        <IconButton icon="close" size="xs" label={t('diagnostics.close')} onClick={() => shell.setDiagnosticsOpen(false)} />
      </header>
      <div className="palette__body">
        {!status ? <p className="palette__empty">{t('diagnostics.empty')}</p> : null}
        <h3 className="palette__section">{t('diagnostics.section.renderer')}</h3>
        <dl className="kv">
          <Row label="backend">{value(status?.backend)}</Row>
          <Row label="adapter">{value(status?.adapter)}</Row>
          <Row label={t('render.fps')}>{shell.fpsText}</Row>
        </dl>
        <h3 className="palette__section">{t('diagnostics.section.document')}</h3>
        <dl className="kv">
          <Row label={t('resources.import.identity')}>{value(status?.documentLabel)}</Row>
          <Row label="entityCount">{value(status?.entityCount)}</Row>
          <Row label="importCount">{value(status?.importCount)}</Row>
          <Row label="camera">{value(status?.camera)}</Row>
        </dl>
        <h3 className="palette__section">{t('diagnostics.section.status')}</h3>
        <dl className="kv">
          <Row label="loading">{value(status?.loading)}</Row>
          <Row label="canOpen">{value(status?.canOpen)}</Row>
          <Row label="canFit">{value(status?.canFit)}</Row>
          <Row label="accepted">{String(shell.commandResult?.kind === 'accepted')}</Row>
        </dl>
        {shell.displayedError ? <pre className="palette__error" role="alert">{shell.displayedError}</pre> : null}
        <Button variant="plain" size="xs" icon="close" onClick={() => shell.setDiagnosticsOpen(false)}>
          {t('diagnostics.close')}
        </Button>
      </div>
    </section>
  );
}
