// Accessible non-visual mirror of the Slint canvas state (audit U12).
//
// Boundary: the Slint `#canvas` has no native HTML semantics — it is a single
// opaque drawing surface. This module does **not** try to re-implement an ARIA
// canvas model (no per-entity roles, no grid navigation). Instead it maintains a
// short, screen-reader-facing sentence in a visually-hidden `role="status"`
// live region, fed from the same authoritative status that `#host-state` shows
// (itself derived from `window.yacr.renderer_state_report()` in `renderer.js`).
//
// Announcing through **one** funnel (here) is deliberate: `#host-state` is a
// technical loading/failure line and is `aria-live="off"`, so the two never
// double-announce the same event.
export function createA11y(i18n) {
  let last = null;

  /// Replace the live region's text. Identical consecutive text is a no-op, so
  /// the 2s heartbeat cannot make a screen reader repeat a stable sentence.
  function announce(text) {
    if (typeof text !== "string" || text.length === 0) return;
    if (text === last) return;
    last = text;
    const node = document.getElementById("a11y-status");
    if (node) node.textContent = text;
  }

  /// Map a host catalog key onto a human sentence for the live region.
  ///
  /// Failure keys normally embed the raw `renderer_state_report()` string; the
  /// live region states the same conclusion without the technical dump, so a
  /// screen reader never reads it.
  function sentenceForStateKey(key, args) {
    if (key === "host.renderer_failed" || key === "host.startup_failed") {
      return i18n.t("a11y.status_failed");
    }
    return i18n.t(key, args);
  }

  /// Announce the localized status already shown in `#host-state`.
  function announceStateKey(key, args = {}) {
    announce(sentenceForStateKey(key, args));
  }

  /// Announce a raw host status string (e.g. the Rust host's open/import line).
  /// It is already human prose, so it is shown verbatim.
  function announceStateText(text) {
    announce(text);
  }

  return {
    announce,
    announceStateKey,
    announceStateText,
    sentenceForStateKey,
    lastAnnounced: () => last,
  };
}
