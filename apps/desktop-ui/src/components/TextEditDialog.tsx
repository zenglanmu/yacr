// TextEditDialog: the modal editor for one text entity's raw DXF string
// (ADR 0021 D4). Rendered inside `PopupOverlay` for `kind: 'textEdit'`; the
// overlay chrome owns the title, the close button and Escape.
//
// The payload is the host's: `text` is the entity's stored string **verbatim**
// (MTEXT markup included — `\P`, braces, `\W…;` — because that is what the file
// holds and what the host will write back) and `multiline` says whether the
// source class accepts line breaks. This component never converts between `\n`
// and `\P`: the host owns that mapping, per source class.
//
// Keyboard rules (see `textEditKeyAction`), deliberately different per mode:
//
//   * multiline (a source class that stores MTEXT markup) — `Enter` inserts a
//     line break, `Ctrl+Enter` / `Cmd+Enter` applies;
//   * single-line (TEXT) — `Enter` applies, mirroring the command line;
//     `Shift+Enter` inserts nothing, because the host refuses a newline for
//     TEXT and offering one would only earn a refusal after the fact;
//   * an IME composition never submits, in either mode.
//
// Escape and Close live in `PopupOverlay` (one owner for the overlay's one
// accelerator); this file only reports what to close. Neither a host refusal nor
// a failed close may cost the user their draft: the draft is local state, and
// the only path that closes the overlay is a **successful** Apply.
//
// Styling note: `style.css` is not part of this change. The field reuses the
// shared `.commandline__input` tokens and the box metrics come from
// `popup-layout.ts`, next to the other dialogs'.
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../host/invoke.ts';
import { Button } from './primitives.tsx';
import { useI18n } from '../context.ts';
import { setSelectedText } from '../tools/textedit.ts';
import type { PopupView } from '../popup/popup.ts';
import {
  POPUP_TEXT_KEYS,
  createTextEditSubmitter,
  textEditApplyOutcome,
  textEditDraftView,
  textEditKeyAction,
  textEditLocalRefusal,
} from '../popup/popup-textedit.ts';
import {
  DIALOG_FOOT,
  DIALOG_TEXT_BODY,
  TEXT_FIELD,
  TEXT_FIELD_FOCUSED,
  TEXT_META,
} from '../popup/popup-layout.ts';

const FIELD_ID = 'cad-textedit-field';
const HINT_ID = 'cad-textedit-hint';
const COUNT_ID = 'cad-textedit-count';
const NOTICE_ID = 'cad-textedit-notice';

const FOOT_NOTE = Object.freeze({
  margin: '0 auto 0 0',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
});

// `--text-dim`, not `--text-muted`: the pixel review measured the hint's
// anti-aliased core at ~4.0:1 at this size. The stronger secondary token reads
// 6.9:1 against the card (against 5.4:1 for the muted one), so 11px copy clears
// 4.5:1 even after the rasteriser blends it, and it matches the token the rest
// of the app uses for secondary text.
const HINT = Object.freeze({ margin: 0, color: 'var(--text-dim)', fontSize: 11, lineHeight: 1.4 });
const COUNT = Object.freeze({
  margin: 0,
  marginLeft: 'auto',
  color: 'var(--text-dim)',
  lineHeight: 1.4,
  fontVariantNumeric: 'tabular-nums',
  whiteSpace: 'nowrap',
});

/** The text-edit modal for one entity's raw string, rendered inside the overlay. */
interface TextEditDialogProps {
  readonly view: Extract<PopupView, { kind: 'textEdit' }>;
  readonly onClose: () => void;
  /** A failed close, shown in the footer; empty when there is none. */
  readonly closeFailure: string;
}

export function TextEditDialog({ view, onClose, closeFailure }: TextEditDialogProps) {
  const { t } = useI18n();
  const multiline = view.multiline === true;
  // The boot literal is the only seed, and it is adopted as-is. No effect ever
  // re-reads the host here: the popup window has no polling, and the host's
  // modal gate guarantees the selection cannot move under the dialog.
  const [draft, setDraft] = useState(() => (typeof view.text === 'string' ? view.text : ''));
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Focus evidence: the field opens focused, and the ring must say so to a
  // reviewer looking at a still frame — the shared `:focus-visible` outline is
  // keyboard-only and was absent from the reviewed grab.
  const [focused, setFocused] = useState(false);
  // The field's latest value. `onChange` is the only writer, so an Apply that
  // lands in the same task as an IME `compositionend` sends what the field
  // really shows rather than a render behind.
  const draftRef = useRef(draft);
  const fieldRef = useRef<HTMLTextAreaElement | null>(null);
  const submitter = useMemo(
    () => createTextEditSubmitter(text => setSelectedText(invoke, text)),
    [],
  );

  // Open with the caret **after** the existing string. A freshly focused field
  // otherwise starts at offset 0, and the first keystroke would prepend to the
  // entity's text without the user noticing. Nothing is selected either, so
  // typing never replaces the content.
  useEffect(() => {
    const field = fieldRef.current;
    if (!field) return;
    // Focus explicitly as well as via `autoFocus`: React's attribute runs at
    // commit, and a window that is still being raised can swallow it — the
    // reviewed frame had no caret and no ring. Focusing here is idempotent.
    field.focus();
    const end = field.value.length;
    field.setSelectionRange(end, end);
  }, []);

  const state = textEditDraftView(draft, multiline);

  const apply = useCallback(() => {
    const current = textEditDraftView(draftRef.current, multiline);
    // A local refusal is shown, never swallowed and never "fixed" by rewriting
    // the draft: over the bound, empty, or a break in a single-line entity.
    const local = textEditLocalRefusal(current, t);
    if (local) {
      setNotice(local);
      return;
    }
    // A second Apply while the first is queued reports busy; it never sends the
    // same text twice.
    if (submitter.isBusy()) {
      setNotice(t(POPUP_TEXT_KEYS.busy));
      return;
    }
    setBusy(true);
    void submitter.submit(draftRef.current).then(result => {
      const outcome = textEditApplyOutcome(result, t);
      if (outcome.done) {
        // Applying succeeded: the host has the edit. Close the window, which
        // the host owns; a failed close surfaces through `closeFailure` below
        // and leaves both buttons live for a retry.
        setBusy(false);
        setNotice(null);
        onClose();
        return;
      }
      setBusy(false);
      // Refusal: the draft stays exactly as typed, and the notice says why.
      setNotice(outcome.message);
    });
  }, [multiline, onClose, submitter, t]);

  const hint = t(multiline ? POPUP_TEXT_KEYS.hintMultiline : POPUP_TEXT_KEYS.hintSingle);
  const count = t(POPUP_TEXT_KEYS.count, { used: state.length, max: state.max });
  const failure = notice ?? closeFailure ?? null;
  const describedBy = [HINT_ID, COUNT_ID, failure ? NOTICE_ID : null].filter(Boolean).join(' ');
  // Over-long and a break in a single-line entity are the two states where the
  // field's content cannot be sent; empty stays clickable so the refusal is
  // shown instead of a mysteriously dead button.
  const applyDisabled = busy || state.tooLong || state.singleLineBreak;

  return (
    <>
      <div style={DIALOG_TEXT_BODY}>
        <textarea
          id={FIELD_ID}
          ref={fieldRef}
          className="commandline__input"
          style={focused ? TEXT_FIELD_FOCUSED : TEXT_FIELD}
          value={draft}
          rows={multiline ? 8 : 3}
          wrap={multiline ? 'soft' : 'off'}
          autoFocus
          autoComplete="off"
          spellCheck={false}
          aria-label={t(POPUP_TEXT_KEYS.title)}
          aria-describedby={describedBy}
          aria-invalid={state.tooLong || state.singleLineBreak || undefined}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onChange={event => {
            draftRef.current = event.target.value;
            setDraft(event.target.value);
            setNotice(null);
          }}
          onKeyDown={event => {
            const action = textEditKeyAction(event, multiline);
            if (action === 'none') return;
            event.preventDefault();
            if (action === 'submit') apply();
          }}
        />
        <div style={TEXT_META}>
          <p id={HINT_ID} style={HINT}>{hint}</p>
          <p
            id={COUNT_ID}
            className="mono"
            style={state.tooLong ? { ...COUNT, color: 'var(--danger)' } : COUNT}
          >
            {count}
          </p>
        </div>
      </div>
      <footer style={DIALOG_FOOT}>
        {failure ? (
          <p id={NOTICE_ID} className="palette__error" role="alert" title={failure} style={FOOT_NOTE}>
            {failure}
          </p>
        ) : null}
        <Button onClick={onClose}>{t(POPUP_TEXT_KEYS.close)}</Button>
        <Button
          variant="solid"
          icon="check"
          iconSize={14}
          disabled={applyDisabled}
          aria-busy={busy || undefined}
          title={`${t(POPUP_TEXT_KEYS.apply)} (Ctrl+Enter)`}
          onClick={apply}
        >
          {t(POPUP_TEXT_KEYS.apply)}
        </Button>
      </footer>
    </>
  );
}
