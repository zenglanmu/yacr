// The left tool-column slot.
//
// `concept` registers the SketchUp Large Tool Set here; a future domain can
// register its own groups without touching the tree. Every cell is the same
// `RibbonItem` the band/menu use, so admission, refusal copy and dispatch are
// the shared ones (`itemEnabled`/`unavailable`/`runItem`).
//
// The one domain-local control is **Select**. SketchUp's Select is the idle
// pointer state, not a router command: with no tool active it is already in
// effect, and with a tool active the honest action is the same `CANCEL` Escape
// sends. It therefore has no `command` token of its own (a `CANCEL` token would
// be a lie about what the button means) and is handled here.
import React from 'react';
import { Icon } from '../../icons.tsx';
import type { IconName } from '../../icons.tsx';
import { useI18n, useShell } from '../../context.ts';
import type { RibbonItem, RibbonPanel } from '../../ribbon/ribbon-model.ts';
import { itemEnabled, runItem, unavailable } from '../../components/Ribbon.tsx';
import { currentDomain } from '../store.ts';
import { useDomainUi } from './use-ui-layout.ts';

/** The id the concept toolbar uses for its one state control. */
export const CONCEPT_SELECT_ID = 'concept.select';

function ToolCell({ item }: { readonly item: RibbonItem }) {
  const { t } = useI18n();
  const shell = useShell();
  const select = item.id === CONCEPT_SELECT_ID;
  const toolActive = Boolean(shell.status?.tool);
  // Select is always operable: it is the idle state, or a request to cancel the
  // active capture. Every other item uses the shared gate.
  const enabled = select || itemEnabled(shell, item);
  const active = select && !toolActive;
  const title = enabled
    ? (item.hintKey ? t(item.hintKey) : t(item.labelKey))
    : unavailable(t, item, shell);
  const onClick = () => runItem(shell, item);
  return (
    <button
      type="button"
      className={['concept-tool', active ? 'is-active' : '', enabled ? '' : 'is-disabled'].filter(Boolean).join(' ')}
      title={title}
      aria-label={t(item.labelKey)}
      aria-pressed={select ? active : undefined}
      aria-disabled={enabled ? undefined : 'true'}
      onClick={enabled ? onClick : undefined}
    >
      <Icon name={item.icon as IconName} size={18} />
    </button>
  );
}

function Group({ group }: { readonly group: RibbonPanel }) {
  const { t } = useI18n();
  return (
    <section className="concept-toolgroup" aria-label={t(group.titleKey)}>
      <h2 className="sr-only">{t(group.titleKey)}</h2>
      <div className="concept-toolgrid">
        {group.items.map(item => <ToolCell key={item.id} item={item} />)}
      </div>
    </section>
  );
}

/** The visual order is the registry's group order, top to bottom. */
export function DomainToolbar() {
  const { t } = useI18n();
  const { status } = useShell();
  const domain = currentDomain(status);
  const layout = useDomainUi(domain);
  if (!layout.toolbar || layout.toolbar.length === 0) return null;
  return (
    <nav className="concept-toolbar" aria-label={t('domain.toolbar_label')}>
      {layout.toolbar.map(group => <Group key={group.id} group={group} />)}
    </nav>
  );
}
