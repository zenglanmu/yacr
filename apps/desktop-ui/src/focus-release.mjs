// Press hand-off for the command line.
//
// While the command line holds focus the CAD pointer lane is suspended: typing
// must never leave a pressed/captured pointer behind on the drawing surface.
// The browser, however, moves focus as the *default action* of a press, which
// runs only after the whole event has been dispatched. A press on the drawing
// area therefore reaches `.native-input` while the lane is still suspended, and
// the click that should return focus is dropped — the user has to click twice.
//
// Releasing the field from a **capture-phase** `pointerdown` listener on the
// window runs before the event reaches `.native-input`, so `blur` re-enables the
// lane in time for that very press. The listener must neither `preventDefault()`
// nor `stopPropagation()`: the same press has to keep travelling, unchanged, to
// the lane. A press whose target is inside the command line keeps focus, so the
// input, completion list and history panel stay usable.
export function releaseFocusOnOutsidePress(root, release, env = window) {
  // No container: installing a release for every press would blur the field on
  // its own controls. Do nothing instead of guessing.
  if (!root) return () => {};
  const handler = event => {
    if (root.contains?.(event.target)) return;
    release();
  };
  env.addEventListener('pointerdown', handler, true);
  return () => env.removeEventListener('pointerdown', handler, true);
}
