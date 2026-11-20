// The pre-save "embed external resources" dialog.
//
// It appears **after** SAVE was admitted and **before** the host opens its save
// picker, and only when `cad_ymodel_resource_preview` listed at least one
// resource (see `commands/save-embed.ts`, which owns that decision). The user
// answers one question — pack the external files into the saved drawing or not —
// and the shell then applies the answer and dispatches SAVE.
//
// Shape: the same in-DOM dialog family as the DWG save confirmation and the ADR
// 0021 popups (`.popup-overlay` backdrop, shared header/body/footer metrics, one
// portal to `document.body`). The two lists are the point of the dialog, so they
// get the room: found rows carry an accent edge and the size they would add,
// missing rows are visibly *information only* — dashed edge, muted name, no size
// — so a file that will not travel can never look embedded. Cancel is the initial
// focus (the safe answer), Escape cancels, and Tab cannot leave the card; the
// trap itself lives in `confirm-focus.ts`.
//
// Every sentence comes from the shared catalogs. The two footer actions are the
// save funnel's own labels: `save.embed.confirm` for the packing save and
// `save.dwg_confirm.cancel` for "stop, save nothing".
import React, { useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../icons.tsx';
import type { IconName } from '../icons.tsx';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { useConfirmDialogFocus } from './confirm-focus.ts';
import {
  DIALOG_BODY,
  DIALOG_FOOT,
  DIALOG_HEAD,
  DIALOG_TITLE,
} from '../popup/popup-layout.ts';
import {
  embedGroups,
  embedInitialToggle,
  embedKindKey,
  embedTotal,
  formatByteSize,
} from '../commands/save-embed.ts';
import type { EmbedPreview, EmbedResourceEntry } from '../commands/save-embed.ts';

/** One row's glyph: the kind's own mark, never a generic file icon. */
const KIND_ICON: Readonly<Record<string, IconName>> = Object.freeze({
  font: 'font',
  xref: 'reference',
  image: 'drawing',
});

export interface SaveEmbedDialogProps {
  readonly preview: EmbedPreview;
  /** A `cad_set_embed_resources` call is in flight. */
  readonly busy: boolean;
  /** The host's refusal text, kept verbatim; `null` when there is none. */
  readonly error: string | null;
  readonly onConfirm: (embed: boolean) => void;
  readonly onCancel: () => void;
}

function ResourceRow({ entry }: { readonly entry: EmbedResourceEntry }) {
  const { t } = useI18n();
  const size = entry.found ? formatByteSize(entry.bytes) : '';
  return (
    <li
      className={`save-embed__row ${entry.found ? 'is-found' : 'is-missing'}`}
      title={entry.source ?? undefined}
    >
      <span className="save-embed__kind">
        <Icon name={KIND_ICON[entry.kind] ?? 'drawing'} size={13} />
        {t(embedKindKey(entry.kind))}
      </span>
      <span className="save-embed__name">{entry.name}</span>
      <span className="save-embed__size">{size}</span>
    </li>
  );
}

function ResourceGroup({
  heading,
  entries,
}: {
  readonly heading: string;
  readonly entries: readonly EmbedResourceEntry[];
}) {
  if (entries.length === 0) return null;
  return (
    <section className="save-embed__group" aria-label={heading}>
      <header className="save-embed__group-head">
        <span>{heading}</span>
        <span className="save-embed__count">{entries.length}</span>
      </header>
      <ul className="save-embed__list">
        {entries.map((entry, index) => (
          <ResourceRow key={`${entry.kind}:${entry.name}:${index}`} entry={entry} />
        ))}
      </ul>
    </section>
  );
}

export function SaveEmbedDialog({ preview, busy, error, onConfirm, onCancel }: SaveEmbedDialogProps) {
  const { t } = useI18n();
  const titleId = useId();
  const leadId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [embed, setEmbed] = useState(() => embedInitialToggle(preview));
  useConfirmDialogFocus({ dialog: dialogRef, initial: cancelRef }, onCancel);

  const groups = embedGroups(preview.entries);
  const total = embedTotal(preview.entries);
  const size = formatByteSize(total.bytes);
  const empty = preview.entries.length === 0;

  const content = (
    <div className="popup-overlay save-embed-overlay">
      <div
        ref={dialogRef}
        className="popup-dialog save-embed"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={leadId}
        tabIndex={-1}
      >
        <header className="palette__head save-embed__head" style={DIALOG_HEAD}>
          <h2 id={titleId} className="save-embed__title" style={DIALOG_TITLE}>
            <Icon name="save" size={15} />
            {t('save.embed.title')}
          </h2>
        </header>
        <div className="save-embed__body" style={DIALOG_BODY}>
          <p id={leadId} className="save-embed__benefit">{t('save.embed.benefit')}</p>
          <p className="save-embed__cost">{t('save.embed.cost')}</p>

          <label className="save-embed__toggle" htmlFor={`${titleId}-embed`}>
            <input
              id={`${titleId}-embed`}
              type="checkbox"
              className="save-embed__switch"
              checked={embed}
              disabled={busy}
              onChange={event => setEmbed(event.target.checked)}
            />
            <span>{t('save.embed.toggle')}</span>
          </label>

          {empty ? (
            <p className="save-embed__empty">{t('save.embed.empty')}</p>
          ) : (
            <div className="save-embed__groups">
              <ResourceGroup heading={t('save.embed.groupFound')} entries={groups.found} />
              <ResourceGroup heading={t('save.embed.groupMissing')} entries={groups.missing} />
            </div>
          )}

          <div className="save-embed__meta">
            {empty ? null : (
              <p className="save-embed__total">{t('save.embed.total', { count: total.count, size })}</p>
            )}
            <p className="save-embed__note">{t('save.embed.ymodelOnly')}</p>
          </div>

          {error ? (
            <p className="palette__error save-embed__error" role="alert">{error}</p>
          ) : null}
        </div>
        <footer className="save-embed__footer" style={DIALOG_FOOT}>
          <Button ref={cancelRef} variant="ghost" disabled={busy} onClick={onCancel}>
            {t('save.dwg_confirm.cancel')}
          </Button>
          <Button
            variant="solid"
            icon="save"
            iconSize={14}
            disabled={busy}
            aria-busy={busy || undefined}
            onClick={() => onConfirm(embed)}
          >
            {t('save.embed.confirm')}
          </Button>
        </footer>
      </div>
    </div>
  );

  return typeof document === 'undefined' ? content : createPortal(content, document.body);
}
