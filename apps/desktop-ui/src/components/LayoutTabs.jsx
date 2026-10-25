// Model / Layout tabs, left-aligned at the lower edge of the drawing area.
// The drawing set exposes one model space this round, so "Model" is the active
// tab and the layout affordances are explicitly unavailable.
//
// Both of them are disabled for a **structural** reason — one layout space per
// drawing — not because a handler is missing. So their tooltip says the
// limitation (`limits.mjs` owns the key) instead of `layout.switch_unwired`
// ("the host provides no space selection"), which would claim a wiring gap where
// the product has a design decision.
import React from 'react';
import { Icon } from '../icons.jsx';
import { useI18n } from '../context.mjs';
import { ONE_LAYOUT_SPACE_KEY } from '../limits.mjs';

export function LayoutTabs() {
  const { t } = useI18n();
  const reason = t(ONE_LAYOUT_SPACE_KEY);
  return (
    <div className="region region--layouttabs layouttabs app-region" aria-label={t('layout.panel')}>
      <div className="layouttabs__list" role="tablist" aria-label={t('layout.panel')}>
        <button type="button" role="tab" aria-selected className="layout-tab is-active">
          {t('layout.tab.model')}
        </button>
        <button type="button" className="layout-tab is-disabled" disabled title={reason}>
          <Icon name="layers" size={13} />
          {t('layout.panel')}
        </button>
      </div>
      <button
        type="button"
        className="layouttabs__add"
        disabled
        title={reason}
        aria-label={t('layout.tab.add')}
      >
        <Icon name="plus" size={13} />
      </button>
    </div>
  );
}
