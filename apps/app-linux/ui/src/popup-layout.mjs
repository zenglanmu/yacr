// Shared inline metrics for the popup dialogs.
//
// `style.css` belongs to the shell and is out of scope for this layer, so the
// dialog keeps its metrics here as data: one content edge, one padding rhythm,
// and the vertical composition. All dialogs (About, the text editor and the
// explicit "unknown kind" state) read these, so their edges can never drift
// apart.
//
// Content edge — the body's mark and the footer's button row put their *box*
// edge at `DIALOG_INSET` (16). The header is icon-led, and a glyph inside a
// 24-viewBox icon does not reach the box edge, so the header (left) and the
// close button (right) are nudged inward by that glyph's own inset to land on
// the same optical edge: 14 + 2.5 and 12 + 4.5 are both 16.5px of ink, half a
// pixel off the body's 16. The pixel review measured 15px header ink vs 21px
// body ink; both now sit on the same edge.
//
// Vertical composition — the window height is owned by the host, so the body
// takes the remaining space and centres its content block instead of leaving a
// void above the footer. `margin: auto 0` on the inner block is the safe form:
// when the content is taller than the body it collapses to 0 and the body
// scrolls, which `justify-content: center` would not do (the top would be
// unreachable).
export const DIALOG_INSET = 16;

/** Ink inset of a 24-viewBox glyph drawn in a 15px box (`app`, header left). */
export const HEADER_GLYPH_INSET = 2.5;

/** Ink inset of the close X drawn in an 18px box (`M6 6l12 12`). */
export const CLOSE_GLYPH_INSET = 4.5;

/** Header left padding: 14 + 2.5 = 16.5px of ink. */
export const HEADER_INSET = 14;

/** Header right padding: 12 + 4.5 = 16.5px of ink — symmetric with the left. */
export const CLOSE_INSET = 12;

export const DIALOG_CARD = Object.freeze({
  background: 'var(--chrome-2)',
  border: '1px solid var(--line-soft)',
  borderRadius: 'var(--radius)',
  overflow: 'hidden',
  animation: 'fade var(--t-med) both',
});

export const DIALOG_HEAD = Object.freeze({
  flex: '0 0 auto',
  height: 30,
  padding: `0 ${CLOSE_INSET}px 0 ${HEADER_INSET}px`,
});

// The dialog's title. One step above the shared panel-title size (12px) so the
// header's ink reads at full strength — the pixel review measured the 12px
// suffix at 2.65:1 while the same token at 15px measured 9.5:1. It stays below
// the 15px brand, and the colour is the full text token (9.5:1 on this card),
// never a muted one: the header carries the dialog's title, not a subtitle.
export const DIALOG_TITLE = Object.freeze({
  margin: 0,
  fontSize: '13px',
  fontWeight: 600,
  letterSpacing: '.2px',
  color: 'var(--text)',
});

export const DIALOG_BODY = Object.freeze({
  flex: '1 1 auto',
  minHeight: 0,
  overflow: 'auto',
  display: 'flex',
  flexDirection: 'column',
  padding: `20px ${DIALOG_INSET}px 18px`,
});

/** The centred content block; `auto 0` margins collapse when it overflows. */
export const DIALOG_CONTENT = Object.freeze({
  margin: 'auto 0',
  display: 'flex',
  flexDirection: 'column',
  gap: 14,
  minWidth: 0,
});

export const DIALOG_FOOT = Object.freeze({
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
// `about` is a small card with a centred block; the text editor is not. The
// host owns a taller window for this kind (and no size is assumed here), so the
// body fills it: the field takes every free pixel and the meta line sits under
// it, on the same 16px content edge as the header and footer. The body's own
// `overflow` is hidden on purpose — the field scrolls its own text, and a
// scrolling body around a growing field would move the footer's buttons.

/**
 * The editor header: taller than the shared one, and the body adds no top
 * padding of its own.
 *
 * The pixel review measured 9px from the card edge to the title's ink and ~20px
 * back down to the field: that second gap was the body's 12px top padding
 * stacked under the header's centring, so the air read as accidental. Making the
 * header 42px and dropping the body's top padding to 0 puts the same 42px above
 * the field, split evenly around the title (~15px of ink on each side) — and the
 * field keeps the exact y it had in the reviewed frame. About keeps the shared
 * 30px header: its body is centred, so it has no such seam to fix.
 */
export const TEXT_HEAD = Object.freeze({ ...DIALOG_HEAD, height: 42 });

/** The editor body: same inset, no centring, no outer scroll, no top padding. */
export const DIALOG_TEXT_BODY = Object.freeze({
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
export const TEXT_FIELD = Object.freeze({
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
export const TEXT_FIELD_FOCUS = Object.freeze({
  borderColor: 'var(--focus)',
  boxShadow: 'inset 0 0 0 1px var(--focus)',
  outline: 'none',
});

/** The field style with the focus ring applied (no layout change). */
export const TEXT_FIELD_FOCUSED = Object.freeze({ ...TEXT_FIELD, ...TEXT_FIELD_FOCUS });

/**
 * Hint (left, secondary ink) and the live `{used} / {max}` count (right).
 *
 * The row wraps only when the copy cannot fit beside the count: the corrected
 * zh-CN hint measures 366px and the count 93px at 11px, well inside the 608px
 * content width of a 640px window. The English hint is 578px, so it keeps one
 * line by moving the count to a second, right-aligned line instead of wrapping
 * mid-sentence.
 */
export const TEXT_META = Object.freeze({
  flex: '0 0 auto',
  display: 'flex',
  alignItems: 'flex-start',
  justifyContent: 'space-between',
  flexWrap: 'wrap',
  gap: '3px 12px',
  minHeight: 16,
});

