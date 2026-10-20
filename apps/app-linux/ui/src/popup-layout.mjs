// Shared inline metrics for the popup dialogs.
//
// `style.css` belongs to the shell and is out of scope for this layer, so the
// dialog keeps its metrics here as data: one content edge, one padding rhythm,
// and the vertical composition. Both dialogs (About and the explicit
// "unknown kind" state) read these, so their edges can never drift apart.
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
