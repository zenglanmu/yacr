// The package surface: what the barrel exports, what `definePlugin` accepts, and
// the source-level rules this package must keep to:
//
//   * strippable syntax only (no enum/namespace declarations);
//   * every relative import carries the `.ts` extension;
//   * the barrel is public surface, not an internal convenience;
//   * the package has **zero host dependencies** — every import in `src/` is
//     relative and stays inside `packages/cad-plugin-api/`.
//
// The source scans read the shipped modules as text on purpose: `tsc` proves the
// files compile, but only a scan can prove the dependency rule, which is a
// repository boundary rather than a TypeScript one.

import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { test } from 'node:test';

import * as surface from '../src/index.ts';
import { API_VERSION, CadApiError, definePlugin } from '../src/index.ts';
import type { CadApiNamespace, CadPlugin } from '../src/index.ts';

const bag = surface as unknown as Record<string, unknown>;
const SRC = new URL('../src/', import.meta.url);
const PACKAGE_ROOT = new URL('../', import.meta.url);
const README = new URL('../README.md', import.meta.url);

type ExportKind = 'function' | 'object' | 'number' | 'string';

const EXPECTED_EXPORTS: readonly (readonly [string, ExportKind])[] = [
  ['API_VERSION', 'number'],
  ['API_NAMESPACES', 'object'],
  ['API_OPERATIONS', 'object'],
  ['operationOf', 'function'],
  ['pendingReason', 'function'],
  ['unsupportedReason', 'function'],
  ['descriptorMissingReason', 'function'],
  ['CadApiError', 'function'],
  ['errorCodeOf', 'function'],
  ['isCadApiError', 'function'],
  ['createCadApi', 'function'],
  ['definePlugin', 'function'],
  ['parsePluginManifest', 'function'],
  ['point2d', 'function'],
  ['point3d', 'function'],
  ['vector2d', 'function'],
  ['vector3d', 'function'],
  ['matrix3dIdentity', 'function'],
  ['matrix3dTranslate', 'function'],
  ['matrix3dScale', 'function'],
  ['matrix3dRotateZ', 'function'],
  ['planeFromOriginNormal', 'function'],
  ['distance3d', 'function'],
  ['addPoints', 'function'],
  ['transformPoint', 'function'],
  ['createGeometryLibrary', 'function'],
  ['aciToRgb', 'function'],
  ['colorFromAci', 'function'],
  ['createColorLibrary', 'function'],
  ['ColorMethod', 'object'],
  ['objectId', 'function'],
  ['handle', 'function'],
  ['Transaction', 'function'],
  ['TransactionManager', 'function'],
  ['MAX_TRANSACTION_DEPTH', 'number'],
  ['SUPPORTED_NEW_ENTITY_TYPES', 'object'],
  ['PromptStatus', 'object'],
  ['selectionSet', 'function'],
];

function renderError(error: unknown): string {
  if (error instanceof CadApiError) return `${error.name}(${error.code})`;
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

function expectSyncCode(run: () => unknown, code: string): CadApiError {
  let caught: unknown;
  try {
    run();
  } catch (error) {
    caught = error;
  }
  if (!(caught instanceof CadApiError)) {
    throw new Error(`expected CadApiError(${code}), got ${renderError(caught)}`);
  }
  assert.equal(caught.code, code);
  return caught;
}

async function sourceFiles(): Promise<readonly (readonly [string, string])[]> {
  const names = await readdir(SRC, { recursive: true });
  const files = names.filter((name) => name.endsWith('.ts')).sort();
  return Promise.all(
    files.map(async (name) => {
      const text = await readFile(new URL(name, SRC), 'utf8');
      return [name, text] as const;
    }),
  );
}

/** Every module specifier in a source file (static import, `export from`, dynamic). */
function importSpecifiers(text: string): readonly string[] {
  const pattern = /(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)['"]([^'"]+)['"]/g;
  return [...text.matchAll(pattern)].map((match) => match[1]);
}

/** Strip comment-only lines so prose cannot trip the keyword scan. */
function stripComments(text: string): string {
  return text
    .split('\n')
    .filter((line) => !/^\s*(\/\/|\*|\/\*)/.test(line))
    .join('\n');
}

test('the barrel exports the whole public surface and nothing is undefined', () => {
  for (const [name, kind] of EXPECTED_EXPORTS) {
    assert.ok(name in bag, `src/index.ts must export ${name}`);
    assert.equal(typeof bag[name], kind, `${name} must be a ${kind}`);
  }
  for (const name of Object.keys(bag)) {
    assert.notEqual(bag[name], undefined, `${name} is exported but undefined`);
  }
  assert.equal(API_VERSION, 1);
});

test('definePlugin returns the same plugin and refuses a broken identity', () => {
  const plugin: CadPlugin = {
    id: 'sample-hello',
    apiVersion: API_VERSION,
    activate: () => undefined,
  };
  assert.equal(definePlugin(plugin), plugin);

  const cases: readonly (readonly [string, CadPlugin])[] = [
    ['bad id', { id: 'Sample Hello', apiVersion: API_VERSION }],
    ['uppercase id', { id: 'Sample', apiVersion: API_VERSION }],
    ['empty id', { id: '', apiVersion: API_VERSION }],
    ['wrong apiVersion', { id: 'sample-hello', apiVersion: API_VERSION + 1 }],
    [
      'non-function activate',
      { id: 'sample-hello', apiVersion: API_VERSION, activate: 'nope' as unknown as () => void },
    ],
    [
      'non-function deactivate',
      { id: 'sample-hello', apiVersion: API_VERSION, deactivate: 3 as unknown as () => void },
    ],
  ];
  for (const [label, candidate] of cases) {
    expectSyncCode(() => definePlugin(candidate), 'api.manifest.invalid');
    assert.ok(label.length > 0);
  }
  expectSyncCode(() => definePlugin(null as unknown as CadPlugin), 'api.manifest.invalid');
});

test('the shipped modules use only strippable TypeScript', async () => {
  const files = await sourceFiles();
  assert.ok(files.length >= 10, `expected the full module set, read ${files.length} file(s)`);

  // `enum`/`namespace` as *declaration keywords*, never as substrings: the
  // contract legitimately talks about namespaces in types and object keys.
  const declarationKeyword =
    /(^|\n)[ \t]*(export[ \t]+)?(declare[ \t]+)?(const[ \t]+)?(enum|namespace)[ \t]+[A-Za-z_$]/;
  const cjk = /[\u3000-\u303F\u3040-\u30FF\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF\uFF00-\uFFEF]/;

  for (const [name, text] of files) {
    const code = stripComments(text);
    assert.equal(declarationKeyword.test(code), false, `${name} must not declare an enum/namespace`);
    assert.equal(cjk.test(text), false, `${name} must not contain CJK text`);
  }

  const readme = await readFile(README, 'utf8');
  assert.equal(cjk.test(readme), false, 'README.md must not contain CJK text');
});

test('every relative import in src carries the .ts extension', async () => {
  const files = await sourceFiles();
  let checked = 0;
  for (const [name, text] of files) {
    for (const request of importSpecifiers(text)) {
      if (!request.startsWith('.')) continue;
      assert.ok(request.endsWith('.ts'), `${name} imports "${request}" without a .ts extension`);
      if (name !== 'index.ts') {
        assert.notEqual(request, './index.ts', `${name} must import concrete modules, not the barrel`);
      }
      checked += 1;
    }
  }
  assert.ok(checked >= 20, `expected the module graph to have imports, saw ${checked}`);
});

test('the package imports nothing outside itself', async () => {
  const files = await sourceFiles();
  const forbidden = ['@tauri-apps', 'react', 'apps/', 'crates/', '@yacr/'];
  const packagePrefix = PACKAGE_ROOT.pathname;
  for (const [name, text] of files) {
    for (const request of importSpecifiers(text)) {
      assert.ok(
        request.startsWith('./') || request.startsWith('../'),
        `${name} imports "${request}": src/ may only use relative imports`,
      );
      for (const needle of forbidden) {
        assert.equal(
          request.includes(needle),
          false,
          `${name} imports "${request}": ${needle} is outside this package`,
        );
      }
      if (request.startsWith('../')) {
        // Walk the specifier against the file's own directory and stay inside the
        // package root: a `../../` escape is how a host dependency sneaks back in.
        const resolved = new URL(request, new URL(name, SRC));
        assert.ok(
          resolved.pathname.startsWith(packagePrefix),
          `${name} imports "${request}", which leaves packages/cad-plugin-api`,
        );
      }
    }
  }
});

test('the descriptor covers namespaces without inventing rows for pure features', () => {
  const namespaces = new Set<CadApiNamespace>();
  for (const row of surface.API_OPERATIONS) namespaces.add(row.namespace);
  for (const namespace of namespaces) {
    assert.ok(
      surface.API_NAMESPACES.includes(namespace),
      `${namespace} must be one of the contract namespaces`,
    );
  }
  assert.equal(namespaces.has('Geometry'), false, 'Geometry helpers are package features');
  assert.equal(namespaces.has('Colors'), false, 'Colors helpers are package features');
});
