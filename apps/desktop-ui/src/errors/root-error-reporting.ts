// React root error reporting.
//
// React 19 calls `onUncaughtError`/`onCaughtError` instead of doing its own
// console logging. We report the render failure into the global dialog and then
// log it ourselves, so the browser console keeps the same raw error a developer
// would otherwise see.
import { reportError } from './error-report.ts';

/** The part of React's `errorInfo` this module reads (structurally typed). */
export interface ReactErrorInfoLike {
  readonly componentStack?: string | null;
}

function stackOf(error: unknown): string | undefined {
  if (error === null || typeof error !== 'object') return undefined;
  const stack = (error as { stack?: unknown }).stack;
  return typeof stack === 'string' && stack.trim() !== '' ? stack : undefined;
}

/** Report one React render error. Never throws; always keeps console output. */
export function reportRenderError(error: unknown, info?: ReactErrorInfoLike): void {
  const message = error instanceof Error ? error.message : String(error);
  const componentStack = typeof info?.componentStack === 'string' && info.componentStack.trim() !== ''
    ? info.componentStack
    : undefined;
  try {
    reportError({ kind: 'render', message, stack: stackOf(error), detail: componentStack });
  } catch {
    // A report store failure must not take the root error path with it.
  }
  console.error(error);
}
