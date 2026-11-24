// Parameters Manager: the document's user-parameter table (ADR-0043 D9).
//
// A non-modal floating palette window on the desktop band and a bottom sheet on
// the compact band — the same component, like the BIM / Font / References panels.
// It never takes layout space, so opening it does not change the CAD partition
// rect and the host never re-presents the drawing.
//
// # What is managed, and what is not
//
// AutoCAD's `PARAMETERS` palette also lists dimensional/geometric **constraint**
// parameters, which only exist because a constraint solver derives them. This
// build implements no such solver, so there are no such rows and the panel says so
// once, in words. What it does manage is the authoritative user-parameter table:
// create a row, replace its value or expression, rename it, delete it. Every edit
// is one `-PARAMETERS …` command through the one merged parametric write path
// (one revision, one undo step), and every refusal is the host's own `param.*`
// reason.
//
// # Why the value field starts empty
//
// `cad_param_list` carries each row's **effective value** (what the table
// evaluates to), not its stored expression. Prefilling the field with that number
// would make one press of Apply silently replace an expression, so the field is a
// fresh input whose placeholder shows the current result. The typed line accepts a
// number or an arithmetic expression over parameter names.
//
// # States that are deliberately different
//
//  * `waiting` — the host has no session yet; still loading, not an empty table.
//  * `empty` — the document really has no user parameters (or none is open; the
//    no-document note is then shown and the create form is disabled).
//  * `error` — the read failed; the host's raw reason is shown, never an empty
//    list.
//  * a refused write keeps the last good rows and shows the reason above them.
import React, { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '../host/invoke.ts';
import { Icon } from '../icons.tsx';
import { Button, IconButton } from './primitives.tsx';
import { useI18n, useShell } from '../context.ts';
import {
  PARAM_KINDS,
  formatParamValue,
  paramKindKey,
  paramStatusKey,
  paramTargetToken,
} from '../params/params.ts';
import type { ParamKindToken, ParamRow } from '../params/params.ts';
import { createParamsStore } from '../params/params-store.ts';
import type { ParamAck, ParamsStore } from '../params/params-store.ts';
import { closeParamsPanel } from '../params/panel.ts';

/** Desktop palette window, or the compact bottom sheet. */
export type ParamsPanelVariant = 'window' | 'floating';

export interface ParamsPanelProps {
  readonly variant?: ParamsPanelVariant;
}

/** One row's value/rename editor. */
function RowEditor({
  row,
  store,
  canWrite,
  onNotice,
}: {
  readonly row: ParamRow;
  readonly store: ParamsStore;
  readonly canWrite: boolean;
  readonly onNotice: (notice: string | null) => void;
}) {
  const { t } = useI18n();
  const [expr, setExpr] = useState('');
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState('');
  const [confirming, setConfirming] = useState(false);
  const unaddressable = paramTargetToken(row.name) === null;

  const report = (ack: ParamAck, invalidKey: string): void => {
    if (ack.kind === 'invalid') onNotice(t(invalidKey));
    else if (ack.kind === 'busy') onNotice(t('parameters.busy'));
    else onNotice(null);
  };

  const applyValue = async (): Promise<void> => {
    if (expr.trim() === '') {
      onNotice(t('parameters.expr_required'));
      return;
    }
    const ack = await store.setValue(row.name, expr.trim());
    if (ack.kind === 'ok') setExpr('');
    report(ack, 'parameters.unaddressable');
  };

  const applyRename = async (): Promise<void> => {
    if (renameDraft.trim() === '') {
      onNotice(t('parameters.name_required'));
      return;
    }
    if (renameDraft.trim() === row.name) {
      setRenaming(false);
      onNotice(null);
      return;
    }
    const ack = await store.rename(row.name, renameDraft.trim());
    if (ack.kind === 'ok') {
      setRenaming(false);
      setRenameDraft('');
    }
    report(ack, 'parameters.unaddressable');
  };

  const applyDelete = async (): Promise<void> => {
    const ack = await store.remove(row.name);
    if (ack.kind === 'ok') setConfirming(false);
    report(ack, 'parameters.unaddressable');
  };

  const kind = paramKindKey(row.kind);
  const status = paramStatusKey(row.status);
  return (
    <li className="params__row">
      <div className="params__row-head">
        <code className="params__name" title={row.name}>{row.name}</code>
        <span className="params__badge" data-status={row.status}>
          {status === null ? row.status : t(status)}
        </span>
      </div>
      <div className="params__meta">
        <span className="params__kind">{kind === null ? row.kind : t(kind)}</span>
        <span className="params__value mono" title={t('parameters.effective_value')}>
          {formatParamValue(row)}
        </span>
        <span className="params__seq">#{row.seq}</span>
      </div>
      {unaddressable ? (
        <p className="params__notice" role="alert">{t('parameters.unaddressable')}</p>
      ) : (
        <div className="params__edit">
          <input
            className="params__input mono"
            type="text"
            value={expr}
            placeholder={formatParamValue(row)}
            aria-label={t('parameters.row.expr', { name: row.name })}
            autoComplete="off"
            spellCheck={false}
            disabled={!canWrite}
            onChange={event => setExpr(event.target.value)}
            onKeyDown={event => {
              if (event.key !== 'Enter' || event.nativeEvent.isComposing) return;
              event.preventDefault();
              void applyValue();
            }}
          />
          <Button size="sm" disabled={!canWrite} onClick={() => { void applyValue(); }}>
            {t('parameters.apply')}
          </Button>
        </div>
      )}
      {unaddressable ? null : renaming ? (
        <div className="params__edit">
          <input
            className="params__input"
            type="text"
            value={renameDraft}
            aria-label={t('parameters.rename.label', { name: row.name })}
            autoComplete="off"
            spellCheck={false}
            disabled={!canWrite}
            onChange={event => setRenameDraft(event.target.value)}
            onKeyDown={event => {
              if (event.key !== 'Enter' || event.nativeEvent.isComposing) return;
              event.preventDefault();
              void applyRename();
            }}
          />
          <Button size="sm" disabled={!canWrite} onClick={() => { void applyRename(); }}>
            {t('parameters.rename')}
          </Button>
          <Button size="sm" onClick={() => { setRenaming(false); onNotice(null); }}>
            {t('parameters.cancel')}
          </Button>
        </div>
      ) : confirming ? (
        <div className="params__actions">
          <Button size="sm" variant="solid" className="params__danger" disabled={!canWrite} onClick={() => { void applyDelete(); }}>
            {t('parameters.confirm_delete')}
          </Button>
          <Button size="sm" onClick={() => setConfirming(false)}>{t('parameters.cancel')}</Button>
        </div>
      ) : (
        <div className="params__actions">
          <Button
            size="sm"
            disabled={!canWrite}
            onClick={() => { setRenaming(true); setRenameDraft(row.name); }}
          >
            {t('parameters.rename')}
          </Button>
          <Button size="sm" variant="solid" className="params__danger" disabled={!canWrite} onClick={() => setConfirming(true)}>
            {t('parameters.delete')}
          </Button>
        </div>
      )}
    </li>
  );
}

/** The create form: one name, one kind, one expression. */
function CreateForm({
  store,
  canWrite,
  onNotice,
}: {
  readonly store: ParamsStore;
  readonly canWrite: boolean;
  readonly onNotice: (notice: string | null) => void;
}) {
  const { t } = useI18n();
  const [name, setName] = useState('');
  const [kind, setKind] = useState<ParamKindToken>('length');
  const [expr, setExpr] = useState('');

  const submit = async (): Promise<void> => {
    if (name.trim() === '') return onNotice(t('parameters.name_required'));
    if (expr.trim() === '') return onNotice(t('parameters.expr_required'));
    const ack = await store.create(name.trim(), kind, expr.trim());
    if (ack.kind === 'ok') {
      setName('');
      setExpr('');
    }
    if (ack.kind === 'invalid') onNotice(t('parameters.unaddressable'));
    else if (ack.kind === 'busy') onNotice(t('parameters.busy'));
    else onNotice(null);
  };

  return (
    <form
      className="params__create"
      onSubmit={event => {
        event.preventDefault();
        void submit();
      }}
    >
      <h3 className="palette__section">{t('parameters.create')}</h3>
      <label className="params__field">
        <span className="params__label">{t('parameters.name')}</span>
        <input
          className="params__input"
          type="text"
          value={name}
          autoComplete="off"
          spellCheck={false}
          disabled={!canWrite}
          onChange={event => setName(event.target.value)}
        />
      </label>
      <label className="params__field">
        <span className="params__label">{t('parameters.kind')}</span>
        <select
          className="params__select"
          value={kind}
          disabled={!canWrite}
          onChange={event => setKind(event.target.value as ParamKindToken)}
        >
          {PARAM_KINDS.map(token => (
            <option key={token} value={token}>{t(`parameters.kind.${token}`)}</option>
          ))}
        </select>
      </label>
      <label className="params__field">
        <span className="params__label">{t('parameters.value')}</span>
        <input
          className="params__input mono"
          type="text"
          value={expr}
          placeholder={t('parameters.value.placeholder')}
          autoComplete="off"
          spellCheck={false}
          disabled={!canWrite}
          onChange={event => setExpr(event.target.value)}
        />
      </label>
      <p className="params__hint">{t('parameters.value.hint')}</p>
      <Button type="submit" size="sm" variant="solid" disabled={!canWrite}>
        {t('parameters.create')}
      </Button>
    </form>
  );
}

export function ParamsPanel({ variant = 'window' }: ParamsPanelProps) {
  const { t } = useI18n();
  const shell = useShell();
  const storeRef = useRef<ParamsStore | null>(null);
  if (!storeRef.current) storeRef.current = createParamsStore(invoke);
  const store = storeRef.current;
  const state = useSyncExternalStore(store.subscribe, store.getState);
  const [notice, setNotice] = useState<string | null>(null);

  // One read per mount: opening the panel is the refresh. A document change
  // remounts it through `Workspace`'s flag, so a stale table is never shown.
  useEffect(() => { void store.load(); }, [store]);

  const canWrite = shell.documentLabel !== null;
  const firstRead = state.phase === 'idle' || state.phase === 'loading';
  const floating = variant === 'floating';
  const active = state.active;
  const close = () => closeParamsPanel();

  return (
    <section
      className={['palette', 'params', floating ? 'params--floating' : ''].filter(Boolean).join(' ')}
      aria-label={t('parameters.title')}
      role={floating ? 'dialog' : undefined}
      aria-modal={floating ? true : undefined}
    >
      <header className="palette__head">
        <h2><Icon name="properties" size={15} />{t('parameters.title')}</h2>
        <IconButton icon="close" size="xs" label={t('parameters.close')} onClick={close} />
      </header>
      <div className="palette__body params__body" aria-busy={firstRead || state.phase === 'waiting' ? true : undefined}>
        {/* The one explicit boundary of this surface: no solver exists, so no
            constraint parameter row exists. */}
        <p className="params__note">{t('parameters.solver_not_modeled')}</p>

        <p className="params__active">
          {active === null
            ? t('parameters.base_table')
            : t('parameters.active_config', { name: active })}
        </p>

        {firstRead || state.phase === 'waiting' ? (
          <p className="palette__empty">{t('parameters.loading')}</p>
        ) : null}

        {state.phase === 'error' ? (
          <p className="palette__error" role="alert">
            {t('parameters.read_failed', { reason: state.error?.reason ?? '' })}
          </p>
        ) : null}

        {!canWrite ? (
          <p className="params__notice" role="status">{t('parameters.no_document')}</p>
        ) : null}

        {state.error?.kind === 'mutation' ? (
          <p className="palette__error" role="alert">
            {t('parameters.write_failed', { reason: state.error.reason })}
          </p>
        ) : null}

        {notice === null ? null : (
          <p className="params__notice" role="alert">{notice}</p>
        )}

        {state.phase === 'empty' && !firstRead ? (
          <p className="palette__empty">{t('parameters.empty')}</p>
        ) : null}

        {state.rows.length > 0 ? (
          <ul className="params__list">
            {state.rows.map(row => (
              <RowEditor
                key={row.seq}
                row={row}
                store={store}
                canWrite={canWrite}
                onNotice={setNotice}
              />
            ))}
          </ul>
        ) : null}

        {state.phase === 'error' && state.rows.length === 0 ? null : (
          <>
            {canWrite ? (
              <CreateForm store={store} canWrite={canWrite} onNotice={setNotice} />
            ) : (
              <p className="params__hint">{t('parameters.no_document_hint')}</p>
            )}
          </>
        )}
      </div>
    </section>
  );
}
