// Minimal ambient declarations for the few Node builtins the suites use.
//
// This package ships no dependencies and the repository has no install step for
// it, so `tsc --noEmit` must not require `@types/node`. Only the surface the
// tests actually touch is declared here — this is not a Node typing and it is
// not shipped (nothing under `src/` imports it).

declare module 'node:test' {
  export interface TestContext {
    skip(message?: string): void;
    diagnostic(message: string): void;
  }
  export function test(
    name: string,
    fn: (context: TestContext) => void | Promise<void>,
  ): Promise<void>;
}

declare module 'node:assert/strict' {
  interface Assert {
    equal(actual: unknown, expected: unknown, message?: string): void;
    notEqual(actual: unknown, expected: unknown, message?: string): void;
    deepEqual(actual: unknown, expected: unknown, message?: string): void;
    notDeepEqual(actual: unknown, expected: unknown, message?: string): void;
    ok(value: unknown, message?: string): void;
    match(value: string, pattern: RegExp, message?: string): void;
    fail(message?: string): never;
  }
  const assert: Assert;
  export default assert;
}

declare module 'node:fs/promises' {
  export function readFile(path: URL | string, encoding: 'utf8'): Promise<string>;
  export function readdir(path: URL | string, options: { recursive: true }): Promise<string[]>;
}

// Node globals the suites use: `import.meta.url` and the WHATWG `URL` (both are
// runtime globals on Node, but neither is in the `ES2022` lib this package
// targets, and pulling in `DOM`/`@types/node` just for them would widen the
// package's own lib contract).
interface ImportMeta {
  readonly url: string;
}

interface URL {
  readonly href: string;
  readonly pathname: string;
}

declare const URL: {
  new (input: string, base?: string | URL): URL;
};
