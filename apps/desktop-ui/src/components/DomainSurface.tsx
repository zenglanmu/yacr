// The domain fallback notice.
//
// A file can report a domain whose **native interface** is not wired in this
// build: the remaining placeholder (`mechanical`) and, defensively, a token this
// build does not know (the host schema rejects those on open, but an older/newer
// host may still send one). The chrome then renders the default
// drafting interface — the most complete one — instead of hiding the ribbon and
// command line behind a dead end.
//
// That fallback is **presentation only**: the document, its `application_domain`
// value and its data are unchanged. This notice is the disclosure that keeps the
// fallback honest; it is a small top banner (not a modal and not a second UI),
// and hiding it changes no state on the wire.
import React, { useEffect, useState } from 'react';
import { useI18n, useShell } from '../context.ts';
import { DOMAIN_PROFILES, unknownDomainToken } from '../domains/domains.ts';
import { currentDomain } from '../domains/store.ts';

export function DomainSurface() {
  const { t } = useI18n();
  const shell = useShell();
  const requested = currentDomain(shell.status);
  const profile = DOMAIN_PROFILES[requested];
  const unknown = unknownDomainToken(shell.status);
  const fallback = unknown !== null || profile.placeholder;
  const [hidden, setHidden] = useState(false);

  // A different requested domain (or a different document) is a new fact: show
  // the notice again instead of carrying one document's dismissal to the next.
  useEffect(() => { setHidden(false); }, [requested, unknown, shell.documentLabel]);

  if (!fallback || hidden) return null;
  const name = unknown ?? t(profile.labelKey);
  return (
    <div className="domain-surface" role="note">
      <p className="domain-surface__title">{t('domain.fallback_title', { domain: name })}</p>
      {unknown !== null ? null : <p className="domain-surface__reference">{t(profile.referenceKey)}</p>}
      <p className="domain-surface__body">
        {unknown !== null
          ? t('domain.fallback_unknown', { domain: unknown })
          : t('domain.fallback_body')}
      </p>
      <button type="button" className="domain-surface__action" onClick={() => setHidden(true)}>
        {t('domain.fallback_dismiss')}
      </button>
    </div>
  );
}
