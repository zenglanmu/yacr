// The plugin message store: the bounded buffer `editor.writeMessage()` fills.
//
// `writeMessage` is **client-local** — it has no descriptor row and no wire call —
// so the client hands the text to a host sink. This module is the WebView's sink:
// it keeps the last few lines and notifies subscribers. There is no React import
// here, so the chrome can subscribe with `useSyncExternalStore` (or a plain
// listener) without this module depending on the chrome.
//
// The cap is deliberate: a plugin that writes in a loop must not grow an unbounded
// buffer, and the oldest line is the one that can go — a truncated log is honest,
// a growing one is a leak.

/** How many lines the store keeps. */
const MAX_MESSAGES = 8;

let messages: readonly string[] = Object.freeze([]);
const listeners = new Set<(messages: readonly string[]) => void>();

/** The retained lines, oldest first (at most {@link MAX_MESSAGES}). */
export function pluginMessages(): readonly string[] {
  return messages;
}

/** Append one line, dropping the oldest past the cap. */
export function appendPluginMessage(text: string): void {
  const next = [...messages, text];
  messages = Object.freeze(next.length > MAX_MESSAGES ? next.slice(-MAX_MESSAGES) : next);
  for (const listener of [...listeners]) listener(messages);
}

/** Subscribe to changes; returns the unsubscribe function. */
export function subscribePluginMessages(
  listener: (messages: readonly string[]) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Test seam: forget every line and tell the subscribers the buffer is empty. */
export function resetPluginMessages(): void {
  messages = Object.freeze([]);
  for (const listener of [...listeners]) listener(messages);
}
