// About dialog: the application name, the version the host reported in the
// popup payload, one honest line about this build, and Close. The shared
// overlay chrome owns the title, the close button and Escape; this file renders
// only the body and footer of the About dialog (ADR 0021 D1, WP3).
//
// Styling: `style.css` is not part of this change, so the dialog reuses the
// existing chrome classes (`.palette__head`, `.mono`, the shared Button
// primitives) and reads its layout metrics from `popup-layout.mjs` — one
// content edge, one padding rhythm, and a vertically centred body, shared with
// the unknown-kind dialog so the two cannot drift.
import React from 'react';
import { Icon } from '../icons.jsx';
import { Button } from './primitives.jsx';
import { useI18n } from '../context.mjs';
import { POPUP_KEYS, aboutVersionText } from '../popup.mjs';
import { DIALOG_BODY, DIALOG_CONTENT, DIALOG_FOOT } from '../popup-layout.mjs';

// The same square mark the title bar uses, in token colours only.
const MARK = {
  display: 'grid',
  placeItems: 'center',
  width: 40,
  height: 40,
  flex: '0 0 auto',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  background: 'var(--field)',
  color: 'var(--accent-text)',
};

export function AboutDialog({ view, onClose, failure }) {
  const { t } = useI18n();
  // The brand is the part of `app.title` before the tagline separator, exactly
  // as the title bar derives it.
  const appName = t('app.title').split(' \u2014 ')[0];
  // One line from the catalog template: both `{version}` and `{build}` are
  // filled from the payload (or an explicit `—`), never left as placeholders.
  const versionLine = aboutVersionText(view, t);
  return (
    <>
      <div style={DIALOG_BODY}>
        <div style={DIALOG_CONTENT}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span style={MARK} aria-hidden="true"><Icon name="app" size={22} /></span>
            <div style={{ minWidth: 0 }}>
              <p style={{ margin: 0, fontSize: 15, fontWeight: 600, letterSpacing: '.2px', color: 'var(--text)' }}>
                {appName}
              </p>
              <p style={{ margin: '3px 0 0', fontSize: 11, color: 'var(--text-muted)', fontVariantNumeric: 'tabular-nums' }}>
                {versionLine}
              </p>
            </div>
          </div>
          <p style={{ margin: 0, color: 'var(--text-dim)', lineHeight: 1.5, maxWidth: '38ch' }}>
            {t(POPUP_KEYS.subtitle)}
          </p>
        </div>
      </div>
      <footer style={DIALOG_FOOT}>
        {failure ? (
          <p
            className="palette__error"
            role="alert"
            style={{ margin: '0 auto 0 0', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
          >
            {failure}
          </p>
        ) : null}
        <Button variant="solid" onClick={onClose}>{t(POPUP_KEYS.close)}</Button>
      </footer>
    </>
  );
}
