// The IME-aware keyboard event the chrome's fields annotate their handlers with.
//
// React's own `KeyboardEvent` type does not declare the native `isComposing`
// flag (the DOM one does), and every IME guard in the chrome reads it. This
// intersection is the one place that flag is stated, so the guards do not cast
// at each read.
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

/** A React keyboard event plus the native IME flag React's type omits. */
export type ComposingKeyEvent<T extends Element = Element> =
  ReactKeyboardEvent<T> & { readonly isComposing?: boolean };
