// File info palette: the static diagnostic state of the open drawing in a
// floating window (non-modal, so the drawing stays live underneath). Every value
// is either real host data or an explicit blank. The live frame rate belongs to
// the diagnostics panel; this one only reports what the file/session is.
import React from 'react';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';

function value(input: unknown): string {
  if (input === null || input === undefined) return '\u2014';
  if (typeof input === 'object') return JSON.stringify(input);
  if (typeof input === 'boolean') return input ? 'true' : 'false';
  return String(input);
}

function Row({ label, children }: { readonly label: React.ReactNode; readonly children: React.ReactNode }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </>
  );
}

export function FileInfoPalette() {
  const { t } = useI18n();
  const shell = useShell();
  const status = shell.status;
  return (
    <section className="palette fileinfo" aria-label={t('fileinfo.title')}>
      <header className="palette__head">
        <h2><Icon name="info" size={15} />{t('fileinfo.title')}</h2>
        <IconButton icon="close" size="xs" label={t('fileinfo.close')} onClick={() => shell.setFileInfoOpen(false)} />
      </header>
      <div className="palette__body">
        {!status ? <p className="palette__empty">{t('fileinfo.empty')}</p> : null}
        <h3 className="palette__section">{t('fileinfo.section.renderer')}</h3>
        <dl className="kv">
          <Row label={t('fileinfo.row.backend')}>{value(status?.backend)}</Row>
          <Row label={t('fileinfo.row.adapter')}>{value(status?.adapter)}</Row>
        </dl>
        <h3 className="palette__section">{t('fileinfo.section.document')}</h3>
        <dl className="kv">
          <Row label={t('resources.import.identity')}>{value(status?.documentLabel)}</Row>
          <Row label={t('fileinfo.row.entities')}>{value(status?.entityCount)}</Row>
          <Row label={t('fileinfo.row.imports')}>{value(status?.importCount)}</Row>
          <Row label={t('fileinfo.row.camera')}>{value(status?.camera)}</Row>
        </dl>
        <h3 className="palette__section">{t('fileinfo.section.status')}</h3>
        <dl className="kv">
          <Row label={t('fileinfo.row.loading')}>{value(status?.loading)}</Row>
          <Row label={t('fileinfo.row.canOpen')}>{value(status?.canOpen)}</Row>
          <Row label={t('fileinfo.row.canFit')}>{value(status?.canFit)}</Row>
          <Row label={t('fileinfo.row.accepted')}>{String(shell.commandResult?.kind === 'accepted')}</Row>
        </dl>
        {shell.displayedError ? <pre className="palette__error" role="alert">{shell.displayedError}</pre> : null}
        <Button
          variant="plain"
          size="xs"
          icon="close"
          className="fileinfo__close"
          onClick={() => shell.setFileInfoOpen(false)}
        >
          {t('fileinfo.close')}
        </Button>
      </div>
    </section>
  );
}
