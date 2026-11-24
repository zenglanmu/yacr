// The placeholder domain card (ADR-0053 D8).
//
// The three non-drafting profiles are explicit placeholders in this build: their
// ribbon and command line are not drawn (a BIM document must not show AutoCAD
// draw tools), and this card says exactly what is missing instead of imitating the
// reference application. It never disables or hides the geometry: the document is
// still the same database and the file commands remain on the start/title chrome.
import React from 'react';
import { useI18n, useShell } from '../context.ts';
import { invoke } from '../host/invoke.ts';
import { DOMAIN_PROFILES, writeDomain } from '../domains/domains.ts';
import { currentDomain, previewDomain } from '../domains/store.ts';

export function DomainSurface() {
  const { t } = useI18n();
  const shell = useShell();
  const domain = currentDomain(shell.status);
  const profile = DOMAIN_PROFILES[domain];
  if (!profile.placeholder) return null;
  const back = () => {
    if (!shell.documentLabel) {
      previewDomain('cad2d');
      return;
    }
    void writeDomain(invoke, 'cad2d');
  };
  return (
    <div className="domain-surface" role="note">
      <p className="domain-surface__title">
        {t('domain.placeholder_title', { domain: t(profile.labelKey) })}
      </p>
      <p className="domain-surface__reference">{t(profile.referenceKey)}</p>
      <p className="domain-surface__body">{t('domain.placeholder_body')}</p>
      <button type="button" className="domain-surface__action" onClick={back}>
        {t('domain.return')}
      </button>
    </div>
  );
}
