// The right default-tray slot.
//
// Each panel names a **built-in body** (`TrayPanel.kind`); a plugin can add a
// declarative `pending` panel with its own sentence, but it cannot inject React.
// That keeps the tray's writes reviewable while still giving extensions a place
// to appear.
//
// The bodies are deliberately read-only or launchers:
//
//   * Entity Info shows the host's real `cad_status.selection` rows and points
//     at the full Properties palette for editing;
//   * Components shows `cad_status.blocks` (name + member count);
//   * Styles hosts the one real appearance switch (the host theme);
//   * Tags opens the existing Tags/Layers palette (`LAYER`), which owns the
//     list and the writes;
//   * Instructor names the active tool and its real next step;
//   * `pending` names the missing panel instead of drawing an empty one.
import React, { useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import { blocksOf } from '../../cad/cad-status.ts';
import { toolLabelKey } from '../../commands/commands.ts';
import { toolStepPromptKey } from '../../cad/cad.ts';
import { propertyRowLabel } from '../../properties/property-labels.ts';
import { ThemeToggle } from '../../components/ThemeToggle.tsx';
import { currentDomain } from '../store.ts';
import { useDomainUi } from './use-ui-layout.ts';
import type { TrayPanel } from './slots.ts';

function EntityInfoBody() {
  const { t } = useI18n();
  const shell = useShell();
  const selection = shell.status?.selection;
  if (!selection || selection.empty) {
    return <p className="concept-panel__hint">{t('concept.entity_info.empty')}</p>;
  }
  return (
    <>
      <p className="concept-panel__hint">{t('concept.entity_info.count', { count: selection.count })}</p>
      <ul className="concept-rows">
        {selection.rows.slice(0, 10).map((row: { readonly key: string; readonly value: string | null; readonly mixed: boolean }) => (
          <li key={row.key}>
            <span className="concept-rows__label">{propertyRowLabel(t, row.key)}</span>
            <span className="concept-rows__value" title={row.value ?? undefined}>
              {row.mixed || row.value === null ? '\u2014' : row.value}
            </span>
          </li>
        ))}
      </ul>
      <button type="button" className="concept-panel__action" onClick={() => shell.runAction('properties')}>
        {t('concept.entity_info.open')}
      </button>
    </>
  );
}

function ComponentsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const blocks = blocksOf(shell.status);
  if (blocks.length === 0) {
    return <p className="concept-panel__hint">{t('concept.components.empty')}</p>;
  }
  return (
    <ul className="concept-rows">
      {blocks.slice(0, 12).map((block, index) => (
        <li key={`${block.name ?? 'unnamed'}-${index}`}>
          <span className="concept-rows__label">{block.name ?? t('classicinsert.unnamed')}</span>
          <span className="concept-rows__value">{t('concept.components.members', { count: block.memberCount })}</span>
        </li>
      ))}
    </ul>
  );
}

function StylesBody() {
  const { t } = useI18n();
  return (
    <div className="concept-panel__styles">
      <p className="concept-panel__hint">{t('concept.styles.theme')}</p>
      <ThemeToggle />
    </div>
  );
}

function TagsBody() {
  const { t } = useI18n();
  const shell = useShell();
  const available = shell.canCommand('LAYER');
  return (
    <>
      <p className="concept-panel__hint">{t('concept.tags.hint')}</p>
      <button
        type="button"
        className="concept-panel__action"
        disabled={!available}
        title={available ? undefined : t('concept.tags.no_document')}
        onClick={() => shell.runCommand('LAYER')}
      >
        {t('concept.tags.open')}
      </button>
    </>
  );
}

function InstructorBody() {
  const { t } = useI18n();
  const shell = useShell();
  const tool = shell.status?.tool ?? null;
  if (!tool) return <p className="concept-panel__hint">{t('concept.instructor.idle')}</p>;
  const step = toolStepPromptKey(tool);
  const nameKey = toolLabelKey(tool.key);
  return (
    <>
      <p className="concept-panel__name">{t(nameKey ?? 'concept.instructor.active')}</p>
      <p className="concept-panel__hint">{step ? t(step) : t('concept.instructor.active')}</p>
    </>
  );
}

function PendingBody({ panel }: { readonly panel: TrayPanel }) {
  const { t } = useI18n();
  return <p className="concept-panel__hint">{t(panel.pendingKey ?? 'concept.pending.panel')}</p>;
}

function Body({ panel }: { readonly panel: TrayPanel }) {
  switch (panel.kind) {
    case 'entityInfo': return <EntityInfoBody />;
    case 'components': return <ComponentsBody />;
    case 'styles': return <StylesBody />;
    case 'tags': return <TagsBody />;
    case 'instructor': return <InstructorBody />;
    case 'scenes':
    case 'pending':
    default: return <PendingBody panel={panel} />;
  }
}

function Section({ panel }: { readonly panel: TrayPanel }) {
  const { t } = useI18n();
  const pending = panel.kind === 'pending' || panel.kind === 'scenes';
  // `open` is controlled by the section's own state so a tap/click toggles it:
  // React's DOM types do not expose `defaultOpen`, and a permanently open body
  // would ignore the user.
  const [open, setOpen] = useState(!pending);
  return (
    <details
      className="concept-tray__section"
      data-kind={panel.kind}
      open={open}
      onToggle={event => setOpen(event.currentTarget.open)}
    >
      <summary className="concept-tray__summary">{t(panel.titleKey)}</summary>
      <div className="concept-tray__body"><Body panel={panel} /></div>
    </details>
  );
}

export function DomainTray() {
  const { t } = useI18n();
  const { status } = useShell();
  const domain = currentDomain(status);
  const layout = useDomainUi(domain);
  if (!layout.tray || layout.tray.length === 0) return null;
  return (
    <aside className="concept-tray" aria-label={t('concept.tray.title')}>
      {layout.tray.map(panel => <Section key={panel.id} panel={panel} />)}
    </aside>
  );
}
