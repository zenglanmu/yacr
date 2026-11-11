// The classic session-group panel (`CLASSICGROUP`).
//
// Rows are the session's real [`EntityGroups`] store projected by `groupRows`:
// name, captured member count and the captured `SelectionRef`s. Groups are
// deliberately **session-only** in this build — they are never DWG objects — so
// the panel says exactly that and lists the stored references.
//
// `Select members` is shown disabled with its stable reason: this build has no
// host path that replaces the CAD selection from a session group, and the panel
// must not fabricate a selection.
//
// The panel is non-modal: no backdrop and no focus trap. It takes the keyboard
// on open, closes on Escape and restores the previous focus.
import React, { useRef, useState } from 'react';
import { Icon } from '../icons.tsx';
import { IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { groupRows } from '../classic/classic-rows.ts';
import type { CadGroupStatus } from '../cad/cad-status.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

interface ClassicGroupPanelProps {
  readonly onClose: () => void;
}

function GroupRow({
  group,
  expanded,
  focused,
  onFocus,
  onToggle,
}: {
  readonly group: CadGroupStatus;
  readonly expanded: boolean;
  readonly focused: boolean;
  readonly onFocus: () => void;
  readonly onToggle: () => void;
}) {
  const { t } = useI18n();
  const rowRef = useRef<HTMLLIElement | null>(null);
  return (
    <li
      ref={rowRef}
      tabIndex={-1}
      className={['references__row', 'is-muted', focused ? 'is-focused' : ''].filter(Boolean).join(' ')}
      aria-current={focused ? 'true' : undefined}
    >
      <div className="references__head">
        <span className="references__name" title={group.name}>{group.name}</span>
        <button
          type="button"
          className="quick-view__badge is-muted"
          onClick={() => { onFocus(); onToggle(); rowRef.current?.focus({ preventScroll: true }); }}
          aria-expanded={expanded}
        >
          {expanded ? t('classicgroup.hideMembers') : t('classicgroup.showMembers')}
        </button>
        <button type="button" disabled title={t('classicgroup.refusal.select')}>
          {t('classicgroup.action.select')}
        </button>
      </div>
      <p className="references__meta">
        <span>{t('classicgroup.memberCount', { count: group.memberCount })}</span>
        <span>{focused ? t('classicgroup.focused') : ''}</span>
      </p>
      {expanded ? (
        group.members.length === 0 ? (
          <p className="references__hint">{t('classicgroup.membersEmpty')}</p>
        ) : (
          <ul className="references__list">
            {group.members.map((member, index) => (
              <li key={`${member.entityId}:${index}`} className="references__field">
                <span className="references__label">{t('classicgroup.memberId')}</span>
                <code className="references__path">{member.entityId}</code>
                {member.instancePath.length === 0 ? null : (
                  <code className="references__path">{member.instancePath.join(' / ')}</code>
                )}
              </li>
            ))}
          </ul>
        )
      ) : null}
    </li>
  );
}

export function ClassicGroupPanel({ onClose }: ClassicGroupPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const { panelRef } = useSurfacePanelFocus(onClose);
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const rows = groupRows(shell.status);
  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="quick-view classic-group"
      role="dialog"
      aria-label={t('classicgroup.title')}
    >
      <header className="quick-view__head">
        <span className="quick-view__title">
          <Icon name="selectAll" size={14} />
          {t('classicgroup.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('resources.close')} onClick={onClose} />
      </header>
      <p className="quick-view__subtitle">{t('classicgroup.subtitle')}</p>
      {rows.length === 0 ? (
        <p className="quick-view__empty">{t('classicgroup.empty')}</p>
      ) : (
        <div className="quick-view__grid">
          <ul className="references__list">
            {rows.map((group, index) => {
              const key = `${group.name}:${index}`;
              return (
                <GroupRow
                  key={key}
                  group={group}
                  expanded={expandedKey === key}
                  focused={focusedKey === key}
                  onFocus={() => setFocusedKey(key)}
                  onToggle={() => setExpandedKey(current => current === key ? null : key)}
                />
              );
            })}
          </ul>
        </div>
      )}
      <footer className="quick-view__foot">
        <p className="references__hint">{t('classicgroup.refusal.select')}</p>
      </footer>
    </aside>
  );
}
