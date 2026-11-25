// The domain switcher: one control in the navigation dock's fixed head.
//
// It is chrome appearance, not a CAD view tool, so it lives beside the theme
// switch in the head rather than in the scrolling rail (the same placement rule
// `NavigationDock` already documents). The popover lists the four profiles with
// the reference application as secondary text; choosing one:
//
// * writes the **document** through `cad_set_application_domain` when a document
//   is open (the next status reports it), or
// * sets a session-only preview when no document is open (never written, never a
//   NEW default).
//
// A refusal (a modal popup, an unknown token, a carrier without the command) is
// shown under the list; the current profile never flips locally.
import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { Icon } from '../icons.tsx';
import { useI18n, useShell } from '../context.ts';
import { invoke } from '../host/invoke.ts';
import {
  DOMAIN_IDS,
  DOMAIN_PROFILES,
  writeDomain,
} from '../domains/domains.ts';
import {
  clearPreviewDomain,
  currentDomain,
  domainStoreSnapshot,
  previewDomain,
  renderedDomain,
  setSwitcherOpen,
  subscribeDomainStore,
  toggleSwitcher,
} from '../domains/store.ts';

/** Apply the rendered domain to `<html data-domain>` (the stylesheet switch). */
export function applyDomainAttribute(domain: string): void {
  if (typeof document === 'undefined') return;
  document.documentElement.dataset.domain = domain;
}

export function DomainSwitcher() {
  const { t } = useI18n();
  const shell = useShell();
  const store = useSyncExternalStore(subscribeDomainStore, domainStoreSnapshot);
  const current = currentDomain(shell.status);
  const rendered = renderedDomain(shell.status);
  const [failure, setFailure] = useState('');

  // The attribute is written before paint so a domain change and its colours land
  // together. A profile whose native interface is not wired falls back to the
  // drafting theme (and the drafting chrome), so the attribute is the **rendered**
  // domain, not the requested one.
  useEffect(() => {
    applyDomainAttribute(rendered);
  }, [rendered]);

  // A document reports its own domain: the document-less preview must not linger
  // into the next welcome page.
  useEffect(() => {
    if (!shell.documentLabel) return;
    // The document is the authority from now on; a welcome-page preview must not
    // reappear when it closes.
    clearPreviewDomain();
    setSwitcherOpen(false);
  }, [shell.documentLabel]);

  const choose = (id: typeof DOMAIN_IDS[number]) => {
    setFailure('');
    setSwitcherOpen(false);
    if (!shell.documentLabel) {
      previewDomain(id);
      return;
    }
    void writeDomain(invoke, id).then(result => {
      if (result.kind === 'failed') {
        setFailure(t('domain.switch_failed', { reason: result.code }));
      }
    });
  };

  const label = t('domain.switcher', { domain: t(DOMAIN_PROFILES[current].labelKey) });
  return (
    <div className="domain-switcher">
      <button
        type="button"
        className="domain-switcher__button"
        aria-haspopup="menu"
        aria-expanded={store.switcherOpen}
        aria-label={label}
        title={label}
        onClick={toggleSwitcher}
      >
        <Icon name="app" size={16} />
      </button>
      {store.switcherOpen ? (
        <>
          <button
            type="button"
            className="domain-switcher__backdrop"
            aria-label={t('domain.close')}
            onClick={() => setSwitcherOpen(false)}
          />
          <div className="domain-switcher__menu" role="menu" aria-label={t('domain.switcher_label')}>
            <p className="domain-switcher__title">{t('domain.switcher_label')}</p>
            {DOMAIN_IDS.map(id => {
              const profile = DOMAIN_PROFILES[id];
              const active = id === current;
              return (
                <button
                  key={id}
                  type="button"
                  role="menuitemradio"
                  aria-checked={active}
                  className={['domain-switcher__row', active ? 'is-on' : ''].filter(Boolean).join(' ')}
                  onClick={() => choose(id)}
                >
                  <span className="domain-switcher__name">{t(profile.labelKey)}</span>
                  <span className="domain-switcher__reference">{t(profile.referenceKey)}</span>
                  {profile.placeholder ? (
                    <span className="domain-switcher__tag">{t('domain.placeholder_tag')}</span>
                  ) : null}
                </button>
              );
            })}
            {!shell.documentLabel ? (
              <p className="domain-switcher__hint">{t('domain.preview_hint')}</p>
            ) : null}
            {failure ? <p className="domain-switcher__error" role="alert">{failure}</p> : null}
          </div>
        </>
      ) : null}
    </div>
  );
}
