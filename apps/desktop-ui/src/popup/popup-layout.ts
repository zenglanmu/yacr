// Shared inline metrics for the popup overlay and its dialogs.
//
// The dialog keeps its metrics here as data: one content edge, one padding
// rhythm, and the vertical composition. The overlay chrome and all three
// dialogs (About, the text editor, Options) read these, so their edges can never
// drift apart. The **card size is UI-owned** now: the popup is an in-DOM
// overlay, so there is no host window whose size it must mirror.
//
// Content edge — the body's mark and the footer's button row put their *box*
// edge at `DIALOG_INSET` (16). The header is icon-led, and a glyph inside a
// 24-viewBox icon does not reach the box edge, so the header (left) and the
// close button (right) are nudged inward by that glyph's own inset to land on
// the same optical edge: 14 + 2.5 and 12 + 4.5 are both 16.5px of ink, half a
// pixel off the body's 16. The pixel review measured 15px header ink vs 21px
// body ink; both now sit on the same edge.
//
// Vertical composition — the card height is set by [`POPUP_CARD_SIZE`] (clamped
// to the viewport), so the body takes the remaining space and centres its
// content block instead of leaving a void above the footer. `margin: auto 0` on
// the inner block is the safe form:
// when the content is taller than the body it collapses to 0 and the body
// scrolls, which `justify-content: center` would not do (the top would be
// unreachable).
import type { CSSProperties } from 'react';

export const DIALOG_INSET = 16;

/** Ink inset of a 24-viewBox glyph drawn in a 15px box (`app`, header left). */
export const HEADER_GLYPH_INSET = 2.5;

/** Ink inset of the close X drawn in an 18px box (`M6 6l12 12`). */
export const CLOSE_GLYPH_INSET = 4.5;

/** Header left padding: 14 + 2.5 = 16.5px of ink. */
export const HEADER_INSET = 14;

/** Header right padding: 12 + 4.5 = 16.5px of ink — symmetric with the left. */
export const CLOSE_INSET = 12;

/** One dialog kind's card size, in logical pixels before viewport clamping. */
export interface PopupCardSize {
  readonly width: number;
  readonly height: number;
}

/** The overlay's card size per kind (logical px, clamped to the viewport). */
export const POPUP_CARD_SIZE = Object.freeze({
  about: Object.freeze({ width: 520, height: 360 }),
  textEdit: Object.freeze({ width: 640, height: 480 }),
  options: Object.freeze({ width: 720, height: 560 }),
} as Readonly<Record<string, PopupCardSize>>);

export const DIALOG_CARD: CSSProperties = Object.freeze({
  background: 'var(--chrome-2)',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  overflow: 'hidden',
  animation: 'fade var(--t-med) both',
});

export const DIALOG_HEAD: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  height: 30,
  padding: `0 ${CLOSE_INSET}px 0 ${HEADER_INSET}px`,
});

// The dialog's title. One step above the shared panel-title size (12px) so the
// header's ink reads at full strength — the pixel review measured the 12px
// suffix at 2.65:1 while the same token at 15px measured 9.5:1. It stays below
// the 15px brand, and the colour is the full text token (9.5:1 on this card),
// never a muted one: the header carries the dialog's title, not a subtitle.
export const DIALOG_TITLE: CSSProperties = Object.freeze({
  margin: 0,
  fontSize: '13px',
  fontWeight: 600,
  letterSpacing: '.2px',
  color: 'var(--text)',
});

export const DIALOG_BODY: CSSProperties = Object.freeze({
  flex: '1 1 auto',
  minHeight: 0,
  overflow: 'auto',
  display: 'flex',
  flexDirection: 'column',
  padding: `20px ${DIALOG_INSET}px 18px`,
});

/** The centred content block; `auto 0` margins collapse when it overflows. */
export const DIALOG_CONTENT: CSSProperties = Object.freeze({
  margin: 'auto 0',
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  minWidth: 0,
});

export const DIALOG_FOOT: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'flex-end',
  gap: 10,
  padding: `10px ${DIALOG_INSET}px`,
  background: 'var(--chrome-3)',
  borderTop: '1px solid var(--line)',
});

// --- Text-edit dialog -----------------------------------------------------
// `about` is a small card with a centred block; the text editor is not. Its
// card is taller (see [`POPUP_CARD_SIZE`]), so the body fills it: the field
// takes every free pixel and the meta line sits under it, on the same 16px
// content edge as the header and footer. The body's own `overflow` is hidden on
// purpose — the field scrolls its own text, and a scrolling body around a
// growing field would move the footer's buttons.

/** The editor body: same inset, no centring, no outer scroll, no top padding. */
export const DIALOG_TEXT_BODY: CSSProperties = Object.freeze({
  ...DIALOG_BODY,
  padding: `0 ${DIALOG_INSET}px 12px`,
  overflow: 'hidden',
  gap: 8,
});

/**
 * The field. Mono, because the content is raw DXF: `\P`, `{`, `}` and `\W…;`
 * are shown as characters, and a proportional face makes them read as mistakes
 * rather than as markup. `resize: none` because the host owns the window size.
 *
 * The horizontal padding is 4px, not 8: the pixel review measured the field's
 * text ink at x27 against the hint's x18, a 9px loose edge. With the 1px border
 * and the mono glyph's own bearing the text now starts at x23, 5px from the
 * hint — a field still reads as a well, but no longer as an indented block.
 */
export const TEXT_FIELD: CSSProperties = Object.freeze({
  flex: '1 1 auto',
  width: '100%',
  height: 'auto',
  minHeight: 96,
  padding: '6px 4px',
  lineHeight: 1.5,
  resize: 'none',
  fontFamily: 'var(--font-mono)',
  tabSize: 4,
});

/**
 * The focused field.
 *
 * The pixel review could not tell the field was focused: its border was the same
 * `--line-soft` as the window frame, and the shared `:focus-visible` ring only
 * appears for keyboard focus (a click into the field shows nothing). This ring is
 * unconditional while the field has focus, drawn **inside** the border box so it
 * shifts no text and clips against no ancestor, and it replaces the shared
 * outline instead of doubling it.
 *
 * `--focus` is the stylesheet's focus token (6.4:1 against `--field`, 3.5:1
 * against the card — a non-text indicator only needs 3:1).
 */
export const TEXT_FIELD_FOCUS: CSSProperties = Object.freeze({
  borderColor: 'var(--focus)',
  boxShadow: 'inset 0 0 0 1px var(--focus)',
  outline: 'none',
});

/** The field style with the focus ring applied (no layout change). */
export const TEXT_FIELD_FOCUSED: CSSProperties = Object.freeze({ ...TEXT_FIELD, ...TEXT_FIELD_FOCUS });

/**
 * Hint (left, secondary ink) and the live `{used} / {max}` count (right).
 *
 * The row wraps only when the copy cannot fit beside the count: the corrected
 * zh-CN hint measures 366px and the count 93px at 11px, well inside the 608px
 * content width of a 640px window. The English hint is 578px, so it keeps one
 * line by moving the count to a second, right-aligned line instead of wrapping
 * mid-sentence.
 */
export const TEXT_META: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  display: 'flex',
  alignItems: 'flex-start',
  justifyContent: 'space-between',
  flexWrap: 'wrap',
  gap: '3px 12px',
  minHeight: 16,
});

// --- Options dialog -------------------------------------------------------
// AutoCAD's Options shape: a left tab rail and one content column, both inside
// the same card, header and footer rhythm as the other dialogs. The card size
// is UI-owned ([`POPUP_CARD_SIZE`]); these constants are the dialog's own
// content arithmetic.
//
// Arithmetic for a 720px window: 156px rail + 1px rule + 2 x 16px inset leaves
// 531px of content; the 148px label column plus a 12px gap leaves 371px for the
// control column, which fits the longest unit row and a `#rrggbb` field at 12px
// mono without horizontal scrolling.
export const OPTIONS_WIDTH = 720;
export const OPTIONS_HEIGHT = 560;
export const OPTIONS_RAIL_WIDTH = 156;

/** Body: the rail and the content column sit side by side, no outer padding. */
export const OPTIONS_BODY: CSSProperties = Object.freeze({
  flex: '1 1 auto',
  minHeight: 0,
  minWidth: 0,
  display: 'flex',
});

/** The tab rail: a fixed column on the deepest chrome surface. */
export const OPTIONS_RAIL: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  width: OPTIONS_RAIL_WIDTH,
  display: 'flex',
  flexDirection: 'column',
  gap: 2,
  padding: '12px 8px',
  overflow: 'auto',
  background: 'var(--chrome-3)',
  borderRight: '1px solid var(--line-soft)',
});

/** One rail tab. Shape comes from `.ui-button` (hover + focus-visible); this
 * only makes it a full-width, left-aligned rail entry. */
export const OPTIONS_TAB: CSSProperties = Object.freeze({
  width: '100%',
  height: 28,
  justifyContent: 'flex-start',
  gap: 8,
  padding: '0 9px',
});

/** The selected tab: raised to the card surface plus a 2px accent edge. */
export const OPTIONS_TAB_ACTIVE: CSSProperties = Object.freeze({
  background: 'var(--chrome-2)',
  color: 'var(--text)',
  boxShadow: 'inset 2px 0 0 var(--accent)',
});

/** The content column: same 16px edge as the header and the footer actions. */
export const OPTIONS_CONTENT: CSSProperties = Object.freeze({
  flex: '1 1 auto',
  minWidth: 0,
  overflow: 'auto',
  display: 'flex',
  flexDirection: 'column',
  padding: `16px ${DIALOG_INSET}px 18px`,
});

/** One settings row: label column, control column, help under the control. */
export const OPTIONS_ROW: CSSProperties = Object.freeze({
  display: 'grid',
  gridTemplateColumns: '148px minmax(0, 1fr)',
  alignItems: 'start',
  gap: '3px 12px',
  padding: '9px 0',
});

/** Hairline between rows: `--line`, not `--line-soft`, so it stays a whisper. */
export const OPTIONS_ROW_RULE: CSSProperties = Object.freeze({ borderTop: '1px solid var(--line)' });

export const OPTIONS_LABEL: CSSProperties = Object.freeze({
  display: 'flex',
  alignItems: 'center',
  minHeight: 24,
  fontSize: 12,
  lineHeight: 1.3,
  color: 'var(--text)',
});

export const OPTIONS_LABEL_INVALID: CSSProperties = Object.freeze({ color: 'var(--danger)' });

/** Help copy uses `--text-dim`, the token the rest of the app uses for secondary
 * text (6.9:1 on the card), not the weaker muted step. */
export const OPTIONS_HELP: CSSProperties = Object.freeze({
  margin: 0,
  fontSize: 11,
  lineHeight: 1.45,
  color: 'var(--text-dim)',
});

export const OPTIONS_HELP_INVALID: CSSProperties = Object.freeze({ color: 'var(--danger)' });

/** One line control: field surface, mono 12px (values are machine-shaped). */
export const OPTIONS_FIELD: CSSProperties = Object.freeze({
  height: 24,
  padding: '0 8px',
  minWidth: 0,
  background: 'var(--field)',
  color: 'var(--text)',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  fontFamily: 'var(--font-mono)',
  fontSize: 12,
});

/** Visible focus ring (inside the border box, no layout shift). */
export const OPTIONS_FIELD_FOCUS: CSSProperties = Object.freeze({
  borderColor: 'var(--focus)',
  boxShadow: 'inset 0 0 0 1px var(--focus)',
  outline: 'none',
});

/** A value the client already knows the server will refuse. */
export const OPTIONS_FIELD_INVALID: CSSProperties = Object.freeze({ borderColor: 'var(--danger)' });

/** Control row: the editor, its unit suffix and (for colours) the swatch. */
export const OPTIONS_CONTROL: CSSProperties = Object.freeze({
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  minWidth: 0,
});

export const OPTIONS_UNIT: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  fontSize: 11,
  color: 'var(--text-muted)',
  fontVariantNumeric: 'tabular-nums',
});

export const OPTIONS_SWATCH: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  width: 22,
  height: 22,
  borderRadius: 2,
  border: '1px solid var(--line-soft)',
  background: 'var(--field)',
});

/** The read-only fonts directory: a field-shaped well, not an editable box. */
export const OPTIONS_WELL: CSSProperties = Object.freeze({
  ...OPTIONS_FIELD,
  display: 'flex',
  alignItems: 'center',
  overflow: 'hidden',
  whiteSpace: 'nowrap',
  textOverflow: 'ellipsis',
  color: 'var(--text-dim)',
});

export const OPTIONS_BADGE: CSSProperties = Object.freeze({
  flex: '0 0 auto',
  display: 'inline-flex',
  alignItems: 'center',
  height: 18,
  padding: '0 6px',
  border: '1px solid var(--line-soft)',
  borderRadius: 999,
  fontSize: 10.5,
  letterSpacing: '.2px',
  color: 'var(--text-muted)',
});

export const OPTIONS_CHECKBOX: CSSProperties = Object.freeze({
  width: 14,
  height: 14,
  margin: 0,
  accentColor: 'var(--accent)',
});

/** Footer staging note (left of the button row), ellipsized like the editors'. */
export const OPTIONS_FOOT_NOTE: CSSProperties = Object.freeze({
  margin: '0 auto 0 0',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
  fontSize: 11,
  color: 'var(--text-dim)',
});

/**
 * The footer refusal band. Unlike the staging note it must be readable in full,
 * so it wraps to two lines at most instead of ellipsizing the one sentence that
 * explains what the host refused.
 */
export const OPTIONS_FOOT_ERROR: CSSProperties = Object.freeze({
  margin: '0 auto 0 0',
  minWidth: 0,
  maxWidth: '52ch',
  fontSize: 11,
  lineHeight: 1.45,
  color: 'var(--danger)',
});

/** The centred loading / load-failure block. */
export const OPTIONS_STATE: CSSProperties = Object.freeze({
  margin: 'auto',
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  gap: 10,
  maxWidth: '46ch',
  textAlign: 'center',
});
