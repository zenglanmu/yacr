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
//   * the mechanical tree reads the host's `cad_mechanical_tree` projection
//     (features then bodies) and never derives geometry itself;
//   * Instructor names the active tool and its real next step;
//   * `pending` names the missing panel instead of drawing an empty one.
import React, { useEffect, useRef, useSyncExternalStore, useState } from 'react';
import { useI18n, useShell } from '../../context.ts';
import { Icon } from '../../icons.tsx';
import { invoke } from '../../host/invoke.ts';
import { blocksOf } from '../../cad/cad-status.ts';
import { toolLabelKey } from '../../commands/commands.ts';
import { toolStepPromptKey } from '../../cad/cad.ts';
import { propertyRowLabel } from '../../properties/property-labels.ts';
import { ThemeToggle } from '../../components/ThemeToggle.tsx';
import { BIM_ACTION } from '../../ribbon/bim-panel.ts';
import { currentDomain, domainStoreSnapshot, subscribeDomainStore, toggleTray } from '../store.ts';
import { useDomainUi } from './use-ui-layout.ts';
import { bodyKindKey, featureOperationKey, mechanicalStatusKey, readMechanicalTree } from '../mechanical/tree.ts';
import type { MechanicalTreeAnswer } from '../mechanical/tree.ts';
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

function BimQueryBody() {
  const { t } = useI18n();
  const shell = useShell();
  return (
    <>
      <p className="concept-panel__hint">{t('bim.tray.query_hint')}</p>
      <button type="button" className="concept-panel__action" onClick={() => shell.runAction(BIM_ACTION)}>
        {t('bim.panel')}
      </button>
    </>
  );
}

/** One tree read's state; `tree` is kept across a failed refresh so the last
 * good projection stays visible (never silently blanked). */
type TreeState =
  | { readonly phase: 'ready'; readonly tree: MechanicalTreeAnswer | null; readonly reason: '' }
  | { readonly phase: 'loading'; readonly tree: MechanicalTreeAnswer | null; readonly reason: '' }
  | { readonly phase: 'error'; readonly tree: MechanicalTreeAnswer | null; readonly reason: string };

/**
 * The FeatureManager design tree.
 *
 * It reads the host's `cad_mechanical_tree` projection — never the drawing
 * itself. A row is labelled from the catalog when the build knows its token and
 * shown raw otherwise; a feature's `schema_only` status is the honest statement
 * that this build stores feature definitions but has no evaluator (ADR-0043 P2).
 * The read refreshes when the document changes, when the entity count changes
 * and after each command, plus the explicit refresh button.
 */
function MechanicalTreeBody() {
  const { t } = useI18n();
  const shell = useShell();
  const documentLabel = shell.status?.documentLabel ?? null;
  const entityCount = shell.status?.entityCount ?? null;
  const commandCount = shell.status?.commandLog?.length ?? 0;
  const [nonce, setNonce] = useState(0);
  const [state, setState] = useState<TreeState>({ phase: 'ready', tree: null, reason: '' });
  // A tree belongs to one document: a refresh keeps the last good projection,
  // but another document must not show the previous one's rows while loading.
  const labelRef = useRef<string | null>(null);

  useEffect(() => {
    const sameDocument = labelRef.current === documentLabel;
    labelRef.current = documentLabel;
    if (documentLabel === null) {
      setState({ phase: 'ready', tree: null, reason: '' });
      return undefined;
    }
    let live = true;
    setState(previous => ({
      phase: 'loading',
      tree: sameDocument ? previous.tree : null,
      reason: '',
    }));
    readMechanicalTree(invoke).then(tree => {
      if (live) setState({ phase: 'ready', tree, reason: '' });
    }).catch((error: unknown) => {
      if (!live) return;
      const reason = error instanceof Error ? error.message : String(error);
      setState(previous => ({ phase: 'error', tree: previous.tree, reason }));
    });
    return () => { live = false; };
  }, [documentLabel, entityCount, commandCount, nonce]);

  if (documentLabel === null) {
    return <p className="concept-panel__hint">{t('mechanical.tree.empty')}</p>;
  }

  const tree = state.tree;
  const rows = tree?.rows ?? [];
  return (
    <>
      <div className="concept-panel__styles">
        <button type="button" className="concept-panel__action" onClick={() => setNonce(value => value + 1)}>
          {t('mechanical.tree.refresh')}
        </button>
      </div>
      {state.phase === 'error' ? (
        <p className="concept-panel__hint" role="alert">{t('mechanical.tree.error', { reason: state.reason })}</p>
      ) : null}
      {rows.length === 0 ? (
        <p className="concept-panel__hint">
          {state.phase === 'loading' ? t('mechanical.tree.loading') : t('mechanical.tree.no_rows')}
        </p>
      ) : (
        <ul className="concept-rows">
          {rows.map(row => {
            const labelKey = row.kind === 'feature' ? featureOperationKey(row.token) : row.kind === 'body' ? bodyKindKey(row.token) : null;
            const statusKey = mechanicalStatusKey(row.status);
            return (
              <li key={`${row.kind}:${row.id}`}>
                <span className="concept-rows__label">{labelKey === null ? row.token : t(labelKey)}</span>
                <span className="concept-rows__value" title={statusKey === null ? row.status : undefined}>
                  {statusKey === null ? row.status : t(statusKey)}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {tree !== null && tree.total > rows.length ? (
        <p className="concept-panel__hint">{t('mechanical.tree.truncated', { shown: rows.length, total: tree.total })}</p>
      ) : null}
    </>
  );
}

function PendingBody({ panel }: { readonly panel: TrayPanel }) {
  const { t } = useI18n();
  return <p className="concept-panel__hint">{panel.pendingText ?? t(panel.pendingKey ?? 'concept.pending.panel')}</p>;
}

function Body({ panel }: { readonly panel: TrayPanel }) {
  switch (panel.kind) {
    case 'entityInfo': return <EntityInfoBody />;
    case 'components': return <ComponentsBody />;
    case 'styles': return <StylesBody />;
    case 'tags': return <TagsBody />;
    case 'instructor': return <InstructorBody />;
    case 'bimQuery': return <BimQueryBody />;
    case 'mechanicalTree': return <MechanicalTreeBody />;
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
      <summary className="concept-tray__summary">{panel.title ?? t(panel.titleKey)}</summary>
      <div className="concept-tray__body"><Body panel={panel} /></div>
    </details>
  );
}

export interface DomainTrayProps {
  /**
   * The side the chrome placed this tray on. The profile owns the choice
   * (`DomainProfile.traySide`): `bim` follows Revit's left dock, `concept` keeps
   * SketchUp's right tray. The component only flips the edge that faces the
   * drawing and stops reserving the right-edge navigation dock's column when it
   * sits on the left.
   */
  readonly side: 'left' | 'right';
}

export function DomainTray({ side }: DomainTrayProps) {
  const { t } = useI18n();
  const { status } = useShell();
  const hidden = useSyncExternalStore(
    subscribeDomainStore,
    () => domainStoreSnapshot().trayHidden,
    () => domainStoreSnapshot().trayHidden,
  );
  const domain = currentDomain(status);
  const layout = useDomainUi(domain);
  if (hidden || !layout.tray || layout.tray.length === 0) return null;
  return (
    <aside
      className={['concept-tray', side === 'left' ? 'concept-tray--left' : ''].filter(Boolean).join(' ')}
      aria-label={t('domain.tray_label')}
    >
      {/* A real title row with the one hide control, the way both reference
          trays have a title bar: the tray is chrome, so the user can put it away
          and bring it back from the profile's own View surface
          (`SHELL_ACTIONS.domainTray`). Sections below keep their own
          collapse; this hides the whole column. */}
      <div className="concept-tray__head">
        <span className="concept-tray__title">{t('domain.tray_label')}</span>
        <button
          type="button"
          className="concept-tray__hide"
          aria-label={t('domain.tray_hide')}
          title={t('domain.tray_hide')}
          onClick={toggleTray}
        >
          <Icon name="close" size={13} />
        </button>
      </div>
      {layout.tray.map(panel => <Section key={panel.id} panel={panel} />)}
    </aside>
  );
}
