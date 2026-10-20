// The one editable control of the inspector: the text content of the single
// selected text entity (ADR 0020 addendum B, option B).
//
// It lives in the docked properties palette — chrome, outside the CAD rectangle
// — because the CAD surface is native and input-transparent: nothing in this
// document may be painted over that rect. The field therefore edits through
// `cad_set_selected_text`, which targets the host's current selection; there is
// no entity id in the payload and no local copy of the drawing.
//
// The value is the host's string **verbatim** (a `textarea`, because an
// `input` strips line breaks), and Apply sends exactly what is in the field. The
// state machine that keeps a draft honest through selection changes lives in
// `textedit.mjs`; this file is layout plus the two event paths (type, apply).
//
// Styling note: `style.css` belongs to another lane in this slice, so the shared
// field surface (`.commandline__input`) supplies the tokens and only the
// palette-specific box is set inline. Moving those three objects into
// `style.css` is the follow-up, not a behaviour change.
import React, { useEffect, useRef, useState } from 'react';
import { Icon } from '../icons.jsx';
import { Button } from './primitives.jsx';
import { useI18n, useShell } from '../context.mjs';
import { TEXT_TOO_LONG_KEY } from '../text.mjs';
import {
  SELECTED_TEXT_MAX_LENGTH,
  TEXT_EDIT_INITIAL,
  TEXT_EDIT_KEYS,
  selectedTextOf,
  textEditApplied,
  textEditDraftChanged,
  textEditHostChanged,
  textEditKeyAction,
  textEditView,
} from '../textedit.mjs';

const EDITOR_STYLE = { display: 'flex', flexDirection: 'column', gap: '4px' };
const LABEL_STYLE = { display: 'inline-flex', alignItems: 'center', gap: '5px' };
const FIELD_STYLE = {
  width: '100%',
  height: 'auto',
  minHeight: '54px',
  padding: '3px 7px',
  lineHeight: 1.4,
  resize: 'vertical',
};
const ACTION_ROW_STYLE = { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '8px' };
const FIELD_ID = 'cad-selection-text';

export function SelectionTextEditor() {
  const { t } = useI18n();
  const shell = useShell();
  const [state, setState] = useState(() => textEditHostChanged(TEXT_EDIT_INITIAL, selectedTextOf(shell.status?.selection ?? null)));
  const [message, setMessage] = useState(null);
  // The text the field last reported. `onChange` is the only writer, so an Apply
  // that lands in the same task as an IME `compositionend` still sends what the
  // field really shows instead of a render behind.
  const draftRef = useRef('');
  const host = selectedTextOf(shell.status?.selection ?? null);
  const view = textEditView(state);

  // Follow the host selection (including the value the palette was opened with,
  // which the initial state already adopted, so the field never pops in a frame
  // late). A draft the user edited is kept — parked with a reason — rather than
  // discarded, and a stale message never outlives its object.
  useEffect(() => {
    setState(current => textEditHostChanged(current, host));
    setMessage(null);
  }, [host]);

  useEffect(() => { draftRef.current = view.value; }, [view.value]);

  function apply() {
    // Detached (no single text selection) or over the bound: the field already
    // says why, and nothing is sent. The button is disabled for the same reason;
    // this guard covers the `Ctrl+Enter` path.
    if (!view.canApply) return;
    const sending = draftRef.current;
    void shell.applySelectedText(sending).then(result => {
      if (!result || result.kind === 'busy') return;
      if (result.kind === 'accepted') {
        // The edit is queued, not finished: the drawing shows the result and any
        // host refusal on the next status. The sent string becomes the baseline
        // so the host's echo is not mistaken for a selection change.
        setState(current => textEditApplied(current, sending));
        setMessage(null);
        return;
      }
      if (result.kind === 'refused') {
        setMessage(result.reason === 'too_long'
          ? t(TEXT_TOO_LONG_KEY, { max: SELECTED_TEXT_MAX_LENGTH })
          : result.reason === 'empty'
            ? t(TEXT_EDIT_KEYS.empty)
            : t(TEXT_EDIT_KEYS.failed, { error: result.error }));
        return;
      }
      setMessage(t(TEXT_EDIT_KEYS.failed, { error: result.error }));
    });
  }

  if (!view.visible) return null;

  // The live bound wins over a stale refusal: it is the reason Apply is off.
  const tooLongText = view.tooLong ? t(TEXT_TOO_LONG_KEY, { max: SELECTED_TEXT_MAX_LENGTH }) : null;
  const notice = tooLongText ?? message;
  const detachedText = view.detached ? t(TEXT_EDIT_KEYS.detached) : null;
  const applyLabel = t(TEXT_EDIT_KEYS.apply);
  const describedBy = [detachedText ? `${FIELD_ID}-detached` : null, notice ? `${FIELD_ID}-notice` : null]
    .filter(Boolean).join(' ');

  return (
    <div className="properties__text" style={EDITOR_STYLE}>
      <label className="palette__section" style={LABEL_STYLE} htmlFor={FIELD_ID}>
        <Icon name="text" size={13} />
        {t(TEXT_EDIT_KEYS.field)}
      </label>
      <textarea
        id={FIELD_ID}
        className="commandline__input"
        style={FIELD_STYLE}
        rows={3}
        value={view.value}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={view.tooLong || undefined}
        aria-describedby={describedBy || undefined}
        onChange={event => {
          draftRef.current = event.target.value;
          setState(current => textEditDraftChanged(current, event.target.value));
          setMessage(null);
        }}
        onKeyDown={event => {
          // A plain Enter is a line break; an IME composition owns its own keys.
          if (textEditKeyAction(event) !== 'submit') return;
          event.preventDefault();
          apply();
        }}
      />
      <div style={ACTION_ROW_STYLE}>
        <Button
          variant="solid"
          size="sm"
          icon="check"
          iconSize={14}
          disabled={!view.canApply}
          title={`${applyLabel} (Ctrl+Enter)`}
          onClick={apply}
        >
          {applyLabel}
        </Button>
        <span
          className="properties__mixed mono"
          style={view.tooLong ? { color: 'var(--danger)' } : undefined}
        >
          {t(TEXT_EDIT_KEYS.count, { used: view.length, max: view.max })}
        </span>
      </div>
      {detachedText ? <p id={`${FIELD_ID}-detached`} className="palette__error" role="status">{detachedText}</p> : null}
      {notice ? <p id={`${FIELD_ID}-notice`} className="palette__error" role="alert">{notice}</p> : null}
    </div>
  );
}
