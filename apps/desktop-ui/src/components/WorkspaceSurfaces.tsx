// The workspace surface: the selector and the settings dialog.
//
// A workspace is the chrome's own docked UI state: ribbon/palette/drawing-tab
// visibility plus the palette panel width. Nothing here reads or writes CAD
// documents, and nothing is written to a file — the preset list lives for this
// session (`chrome/workspaces.ts`).
//
// The selector is **non-modal** (a small floating panel like Quick View); the
// settings dialog is modal (it edits names and can delete a preset), with the
// shared focus trap and Escape close. `WSSAVE` opens the dialog in save mode with
// the name field focused; `WSSETTINGS` opens it in list mode.
import React, { useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import { PALETTE_DEFAULT_WIDTH } from '../layout/palette-window.ts';
import { useWorkspaceSnapshot } from '../chrome/use-chrome-store.ts';
import {
  WORKSPACE_PANEL_MAX_WIDTH,
  WORKSPACE_PANEL_MIN_WIDTH,
  clampWorkspaceWidth,
  deleteWorkspace,
  renameWorkspace,
  requestApplyWorkspace,
  saveWorkspace,
  workspaceName,
  workspacePreset,
} from '../chrome/workspaces.ts';
import type { WorkspaceDialogMode } from '../chrome/surfaces.ts';
import { openWorkspaceSettings } from '../chrome/surfaces.ts';
import {
  DIALOG_BODY,
  DIALOG_CARD,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';
import { useConfirmDialogFocus } from './confirm-focus.ts';
import { useSurfacePanelFocus } from './surface-focus.ts';

const DIALOG_CARD_WORKSPACE: React.CSSProperties = Object.freeze({
  ...DIALOG_CARD,
  display: 'flex',
  flexDirection: 'column',
  width: 'min(520px, calc(100vw - 32px))',
  maxHeight: 'calc(100vh - 32px)',
  boxShadow: '0 18px 48px rgba(0, 0, 0, .45)',
});

// --- Selector -------------------------------------------------------------

interface WorkspaceSelectorProps {
  readonly onClose: () => void;
}

/** The floating workspace list (non-modal, Escape closes). */
export function WorkspaceSelector({ onClose }: WorkspaceSelectorProps) {
  const { t } = useI18n();
  const snapshot = useWorkspaceSnapshot();
  const { panelRef } = useSurfacePanelFocus(onClose);

  return (
    <aside
      ref={panelRef}
      tabIndex={-1}
      className="workspace-panel"
      role="dialog"
      aria-label={t('workspace.title')}
    >
      <header className="workspace-panel__head">
        <span className="workspace-panel__title">
          <Icon name="app" size={14} />
          {t('workspace.title')}
        </span>
        <IconButton icon="close" size="xs" label={t('workspace.close')} onClick={onClose} />
      </header>
      <p className="workspace-panel__hint">{t('workspace.hint')}</p>
      <ul className="workspace-panel__list" role="list">
        {snapshot.presets.map(preset => {
          const current = preset.id === snapshot.currentId;
          return (
            <li key={preset.id} className="workspace-panel__row">
              <button
                type="button"
                className={['workspace-panel__choice', current ? 'is-current' : ''].filter(Boolean).join(' ')}
                title={t('workspace.select')}
                onClick={() => {
                  requestApplyWorkspace(preset.id);
                  onClose();
                }}
              >
                <Icon name={preset.builtin ? 'app' : 'save'} size={14} />
                <span className="workspace-panel__name">{workspaceName(preset, t)}</span>
                {current ? <span className="workspace-panel__badge">{t('workspace.current')}</span> : null}
                {preset.builtin ? <span className="workspace-panel__badge is-muted">{t('workspace.settings.builtin')}</span> : null}
              </button>
            </li>
          );
        })}
      </ul>
      <footer className="workspace-panel__foot">
        <Button
          variant="chip"
          size="sm"
          icon="settings"
          iconSize={13}
          onClick={() => openWorkspaceSettings('settings')}
        >
          {t('workspace.settings')}
        </Button>
        <Button
          variant="chip"
          size="sm"
          icon="save"
          iconSize={13}
          onClick={() => openWorkspaceSettings('save')}
        >
          {t('workspace.save_current')}
        </Button>
      </footer>
    </aside>
  );
}

// --- Settings dialog ------------------------------------------------------

interface WorkspaceSettingsDialogProps {
  readonly mode: WorkspaceDialogMode;
  readonly onClose: () => void;
}

/** The modal workspace manager: save the current state, rename/delete/apply. */
export function WorkspaceSettingsDialog({ mode, onClose }: WorkspaceSettingsDialogProps) {
  const { t } = useI18n();
  const shell = useShell();
  const snapshot = useWorkspaceSnapshot();
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const nameRef = useRef<HTMLInputElement | null>(null);
  const [name, setName] = useState('');
  const [width, setWidth] = useState(String(PALETTE_DEFAULT_WIDTH));
  const [errorKey, setErrorKey] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [deleting, setDeleting] = useState<string | null>(null);
  const [savedName, setSavedName] = useState('');
  useConfirmDialogFocus({ dialog: dialogRef, initial: nameRef }, onClose);

  const fail = (reason: string) => setErrorKey(`workspace.settings.error.${reason}`);

  function submitSave(): void {
    setErrorKey(null);
    const result = saveWorkspace(name, shell.chromeVisibility, clampWorkspaceWidth(width));
    if (!result.ok) {
      fail(result.reason);
      return;
    }
    setSavedName(workspacePreset(result.id)?.name ?? '');
    setName('');
    setWidth(String(PALETTE_DEFAULT_WIDTH));
    nameRef.current?.focus({ preventScroll: true });
  }

  function submitRename(id: string): void {
    setErrorKey(null);
    const result = renameWorkspace(id, renameDraft);
    if (!result.ok) {
      fail(result.reason);
      return;
    }
    setRenaming(null);
    setRenameDraft('');
  }

  function submitDelete(id: string): void {
    setErrorKey(null);
    if (!deleteWorkspace(id)) {
      fail('builtin');
      return;
    }
    setDeleting(null);
  }

  const body = (
    <div className="popup-overlay chrome-dialog-overlay" data-yacr-dialog="workspace-settings" data-mode={mode}>
      <div
        ref={dialogRef}
        className="popup-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t('workspace.settings.title')}
        style={DIALOG_CARD_WORKSPACE}
      >
        <header className="palette__head" style={DIALOG_HEAD}>
          <h2 style={DIALOG_TITLE}>
            <Icon name="settings" size={15} />
            {t('workspace.settings.title')}
          </h2>
          <IconButton icon="close" size="xs" label={t('workspace.settings.close')} onClick={onClose} />
        </header>
        <div style={DIALOG_BODY}>
          <section className="workspace-settings">
            <h3 className="workspace-settings__heading">{t('workspace.settings.save_heading')}</h3>
            <div className="workspace-settings__form">
              <label className="workspace-settings__field">
                <span>{t('workspace.settings.name')}</span>
                <input
                  ref={nameRef}
                  className="select"
                  value={name}
                  placeholder={t('workspace.settings.name_placeholder')}
                  onChange={event => setName(event.target.value)}
                  onKeyDown={event => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      submitSave();
                    }
                  }}
                />
              </label>
              <label className="workspace-settings__field is-narrow">
                <span>{t('workspace.settings.width')}</span>
                <input
                  className="select"
                  type="number"
                  min={WORKSPACE_PANEL_MIN_WIDTH}
                  max={WORKSPACE_PANEL_MAX_WIDTH}
                  step={10}
                  value={width}
                  onChange={event => setWidth(event.target.value)}
                />
              </label>
              <Button variant="chip" icon="save" iconSize={14} onClick={submitSave}>
                {t('workspace.settings.save')}
              </Button>
            </div>
            {savedName !== '' ? (
              <p className="workspace-settings__ok" role="status">{t('workspace.settings.saved', { name: savedName })}</p>
            ) : null}
            {errorKey !== null ? (
              <p className="workspace-settings__error" role="alert">{t(errorKey)}</p>
            ) : null}
          </section>

          <section className="workspace-settings">
            <h3 className="workspace-settings__heading">{t('workspace.settings.list_heading')}</h3>
            <ul className="workspace-settings__list" role="list">
              {snapshot.presets.map(preset => {
                const current = preset.id === snapshot.currentId;
                const label = workspaceName(preset, t);
                return (
                  <li key={preset.id} className="workspace-settings__row">
                    <div className="workspace-settings__identity">
                      <span className="workspace-settings__name">{label}</span>
                      {current ? <span className="workspace-panel__badge">{t('workspace.current')}</span> : null}
                      {preset.builtin ? <span className="workspace-panel__badge is-muted">{t('workspace.settings.builtin')}</span> : null}
                      <span className="workspace-settings__width">{preset.panelWidth}px</span>
                    </div>
                    {renaming === preset.id ? (
                      <div className="workspace-settings__actions">
                        <input
                          className="select"
                          value={renameDraft}
                          aria-label={t('workspace.settings.name')}
                          onChange={event => setRenameDraft(event.target.value)}
                          onKeyDown={event => {
                            if (event.key === 'Enter') {
                              event.preventDefault();
                              submitRename(preset.id);
                            }
                          }}
                        />
                        <Button variant="chip" size="sm" onClick={() => submitRename(preset.id)}>
                          {t('workspace.settings.rename_save')}
                        </Button>
                        <Button variant="plain" size="sm" onClick={() => { setRenaming(null); setErrorKey(null); }}>
                          {t('workspace.settings.rename_cancel')}
                        </Button>
                      </div>
                    ) : deleting === preset.id ? (
                      <div className="workspace-settings__actions">
                        <span className="workspace-settings__confirm">{t('workspace.settings.delete_confirm', { name: label })}</span>
                        <Button variant="chip" size="sm" onClick={() => submitDelete(preset.id)}>
                          {t('workspace.settings.delete_yes')}
                        </Button>
                        <Button variant="plain" size="sm" onClick={() => setDeleting(null)}>
                          {t('workspace.settings.delete_no')}
                        </Button>
                      </div>
                    ) : (
                      <div className="workspace-settings__actions">
                        {!current ? (
                          <Button
                            variant="chip"
                            size="sm"
                            icon="check"
                            iconSize={13}
                            onClick={() => requestApplyWorkspace(preset.id)}
                          >
                            {t('workspace.settings.apply')}
                          </Button>
                        ) : null}
                        {preset.builtin ? null : (
                          <>
                            <Button
                              variant="plain"
                              size="sm"
                              icon="pencil"
                              iconSize={13}
                              onClick={() => {
                                setRenaming(preset.id);
                                setRenameDraft(label);
                                setErrorKey(null);
                              }}
                            >
                              {t('workspace.settings.rename')}
                            </Button>
                            <Button
                              variant="plain"
                              size="sm"
                              icon="trash"
                              iconSize={13}
                              onClick={() => { setDeleting(preset.id); setErrorKey(null); }}
                            >
                              {t('workspace.settings.delete')}
                            </Button>
                          </>
                        )}
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>
            {snapshot.presets.length <= 1 ? (
              <p className="workspace-settings__empty">{t('workspace.settings.empty')}</p>
            ) : null}
          </section>
        </div>
        <footer style={DIALOG_FOOT}>
          <p className="workspace-settings__note">{t('workspace.settings.hint')}</p>
          <Button variant="solid" onClick={onClose}>{t('workspace.settings.close')}</Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? body : createPortal(body, document.body);
}
