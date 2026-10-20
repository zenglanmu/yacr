// The one editable control of the inspector: the text content of the single
// selected text entity (ADR 0020 addendum B, option B). Its slower sibling —
// the host-window editor with room for long text and raw markup — is opened
// from the action row below (ADR 0021 D4); this field stays the fast path.
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
// The keys follow the entity's line-break mode, which the host reports as
// `text_multiline` (`selectedTextMultilineOf`): `Enter` applies for TEXT and
// stays a line break for the multi-line source classes, exactly like the popup
// window. A host that does not send the field yet reads as "unknown" and keeps
// the old keys — the mode is never guessed from a display row.
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
import { POPUP_TEXT_KEYS, canOpenTextEditPopup } from '../popup-textedit.mjs';
import {
  SELECTED_TEXT_MAX_LENGTH,
  TEXT_EDIT_INITIAL,
  TEXT_EDIT_KEYS,
  selectedTextMultilineOf,
  selectedTextOf,
  textEditApplied,
  textEditDraftChanged,
  textEditHostChanged,
  textEditKeyAction,
  textEditView,
} from '../textedit.mjs';

const EDITOR_STYLE = { display: 'flex', flexDirection: 'column', gap: '4px' };
// The section label and the window entry share one row: the entry is an action
// *about* this editor, so it belongs with its heading rather than crowding the
// Apply/count row at the palette's 300px width.
const LABEL_ROW_STYLE = { display: 'flex', alignItems: 'center', flexWrap: 'wrap', justifyContent: 'space-between', gap: '8px', margin: '10px 0 4px' };
const LABEL_STYLE = { display: 'inline-flex', alignItems: 'center', gap: '5px', margin: 0, whiteSpace: 'nowrap' };
const FIELD_STYLE = {
  width: '100%',
  height: 'auto',
  minHeight: '54px',
  padding: '3px 7px',
  lineHeight: 1.4,
  resize: 'vertical',
};
const ACTION_ROW_STYLE = { display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '8px' };
const COUNT_STYLE = { textAlign: 'right' };
const HINT_STYLE = { margin: 0, color: 'var(--text-muted)', fontSize: 11, lineHeight: 1.4 };
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
  const canOpenInWindow = canOpenTextEditPopup(shell.status?.selection ?? null);
  // The entity's line-break policy, straight from the host's `text_multiline`.
  // `null` means "the host does not say" (a build that predates the field) and
  // the field keeps its previous keys; it is never derived from a display row's
  // `type`, because those values are copy, not behaviour.
  const multiline = selectedTextMultilineOf(shell.status?.selection ?? null);
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
    // this guard covers the keyboard path.
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
  // Single-line only: the new Enter binding needs one line of explanation, and
  // the popup already words it ("line breaks are not allowed") — reusing that
  // key keeps the two editors saying the same thing. Multi-line and unknown keep
  // the old affordance untouched.
  const hint = multiline === false ? t(POPUP_TEXT_KEYS.hintSingle) : null;
  const hintId = `${FIELD_ID}-hint`;
  // The accelerator promised on the button matches the keys above: `Enter`
  // applies for single-line text (Ctrl+Enter still works), while a multi-line or
  // unknown draft applies on the explicit Ctrl/Cmd+Enter chord.
  const applyTitle = multiline === false ? `${applyLabel} (Enter)` : `${applyLabel} (Ctrl+Enter)`;
  const describedBy = [
    hint ? hintId : null,
    detachedText ? `${FIELD_ID}-detached` : null,
    notice ? `${FIELD_ID}-notice` : null,
  ].filter(Boolean).join(' ');

  return (
    <div className="properties__text" style={EDITOR_STYLE}>
      <div style={LABEL_ROW_STYLE}>
        <label className="palette__section" style={LABEL_STYLE} htmlFor={FIELD_ID}>
          <Icon name="text" size={13} />
          {t(TEXT_EDIT_KEYS.field)}
        </label>
        {/* The slow path to the same edit: a host window with room for long
            text and for the raw markup. Enabled only when `selection.text` is a
            real string — the host derives the payload from that selection and
            refuses anything else with `popup.no_text_selection`, so an entry
            without text could only produce a refusal. The inline field and
            TEXTEDIT stay exactly as they are: this is an extra affordance, not
            a replacement. */}
        <Button
          variant="ghost"
          size="xs"
          style={{ marginLeft: 'auto' }}
          disabled={!canOpenInWindow}
          onClick={() => shell.openTextEditPopup()}
        >
          {t(POPUP_TEXT_KEYS.entry)}
        </Button>
      </div>
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
          // The popup's key map, in the docked field: single-line text applies on
          // Enter and does nothing on Shift/Alt+Enter; multi-line keeps Enter as
          // a line break and applies on Ctrl/Cmd+Enter; an IME composition owns
          // its own keys in both modes. `block` still prevents the default, so a
          // Shift+Enter inserts no break the host would refuse.
          const action = textEditKeyAction(event, multiline);
          if (action === 'none') return;
          event.preventDefault();
          if (action === 'submit') apply();
        }}
      />
      <div style={ACTION_ROW_STYLE}>
        <Button
          variant="solid"
          size="sm"
          icon="check"
          iconSize={14}
          disabled={!view.canApply}
          title={applyTitle}
          onClick={apply}
        >
          {applyLabel}
        </Button>
        <span
          className="properties__mixed mono"
          style={view.tooLong ? { ...COUNT_STYLE, color: 'var(--danger)' } : COUNT_STYLE}
        >
          {t(TEXT_EDIT_KEYS.count, { used: view.length, max: view.max })}
        </span>
      </div>
      {hint ? <p id={hintId} style={HINT_STYLE}>{hint}</p> : null}
      {detachedText ? <p id={`${FIELD_ID}-detached`} className="palette__error" role="status">{detachedText}</p> : null}
      {notice ? <p id={`${FIELD_ID}-notice`} className="palette__error" role="alert">{notice}</p> : null}
    </div>
  );
}
