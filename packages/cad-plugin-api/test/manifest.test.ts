// Manifest parsing: the pure gate between a plugin directory and a loaded module.
//
// Every refusal is asserted by its exact stable code (`api.manifest.*`), because
// a host will key its diagnostics off the code, not the message text.
//
// The parser takes *text*: these tests never touch the filesystem, which is the
// point of the signature — there is no path resolution, no symlink handling and
// no ambient working directory in the contract.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { API_VERSION, CadApiError, parsePluginManifest } from '../src/index.ts';

function renderError(error: unknown): string {
  if (error instanceof CadApiError) return `${error.name}(${error.code}): ${error.message}`;
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

function expectManifestCode(run: () => unknown, code: string): CadApiError {
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

function manifestText(overrides: Readonly<Record<string, unknown>> = {}): string {
  return JSON.stringify({
    manifestVersion: 1,
    id: 'sample-hello',
    name: 'Sample Hello',
    version: '1.2.3',
    apiVersion: API_VERSION,
    entry: 'dist/index.js',
    commands: ['HELLO_WORLD'],
    ...overrides,
  });
}

test('a complete manifest parses into a frozen value', () => {
  const manifest = parsePluginManifest(manifestText());
  assert.equal(Object.isFrozen(manifest), true);
  assert.deepEqual(
    {
      manifestVersion: manifest.manifestVersion,
      id: manifest.id,
      name: manifest.name,
      version: manifest.version,
      apiVersion: manifest.apiVersion,
      entry: manifest.entry,
      commands: manifest.commands,
    },
    {
      manifestVersion: 1,
      id: 'sample-hello',
      name: 'Sample Hello',
      version: '1.2.3',
      apiVersion: API_VERSION,
      entry: 'dist/index.js',
      commands: ['HELLO_WORLD'],
    },
  );
});

test('commands are optional and the entry may be .mjs', () => {
  const manifest = parsePluginManifest(
    manifestText({ commands: undefined, entry: 'plugin.mjs' }),
  );
  assert.equal('commands' in manifest, false, 'an omitted command list must stay omitted');
  assert.equal(manifest.entry, 'plugin.mjs');
});

test('malformed JSON and non-object roots are refused', () => {
  expectManifestCode(() => parsePluginManifest('{"manifestVersion":1,'), 'api.manifest.invalid_json');
  expectManifestCode(() => parsePluginManifest('[]'), 'api.manifest.not_object');
  expectManifestCode(() => parsePluginManifest('null'), 'api.manifest.not_object');
  expectManifestCode(() => parsePluginManifest('"sample"'), 'api.manifest.not_object');
  expectManifestCode(
    () => parsePluginManifest(null as unknown as string),
    'api.invalid_argument: text',
  );
});

test('field refusals carry exact stable codes', () => {
  const cases: readonly (readonly [string, string, string])[] = [
    ['manifestVersion 2', manifestText({ manifestVersion: 2 }), 'api.manifest.manifest_version'],
    ['missing manifestVersion', manifestText({ manifestVersion: undefined }), 'api.manifest.manifest_version'],
    ['uppercase id', manifestText({ id: 'Sample-Hello' }), 'api.manifest.id'],
    ['id with a space', manifestText({ id: 'sample hello' }), 'api.manifest.id'],
    ['id too long', manifestText({ id: `a${'b'.repeat(64)}` }), 'api.manifest.id'],
    ['empty name', manifestText({ name: '   ' }), 'api.manifest.name'],
    ['non-string name', manifestText({ name: 7 }), 'api.manifest.name'],
    ['empty version', manifestText({ version: '' }), 'api.manifest.version'],
    ['apiVersion 2', manifestText({ apiVersion: 2 }), 'api.manifest.api_version'],
    ['apiVersion string', manifestText({ apiVersion: '1' }), 'api.manifest.api_version'],
    ['non-js entry', manifestText({ entry: 'dist/index.txt' }), 'api.manifest.entry'],
    ['extensionless entry', manifestText({ entry: 'dist/index' }), 'api.manifest.entry'],
    ['parent traversal', manifestText({ entry: '../outside.js' }), 'api.manifest.entry'],
    ['nested traversal', manifestText({ entry: 'dist/../../outside.js' }), 'api.manifest.entry'],
    ['absolute entry', manifestText({ entry: '/opt/plugins/index.js' }), 'api.manifest.entry'],
    ['windows absolute entry', manifestText({ entry: 'C:/plugins/index.js' }), 'api.manifest.entry'],
    ['backslash entry', manifestText({ entry: 'dist\\index.js' }), 'api.manifest.entry'],
    ['unknown key', manifestText({ extra: true }), 'api.manifest.unknown_key: extra'],
    ['commands not an array', manifestText({ commands: 'HELLO_WORLD' }), 'api.manifest.commands'],
    ['lowercase command', manifestText({ commands: ['hello_world'] }), 'api.manifest.commands'],
    ['command with a dash', manifestText({ commands: ['HELLO-WORLD'] }), 'api.manifest.commands'],
    ['duplicate command', manifestText({ commands: ['HELLO', 'HELLO'] }), 'api.manifest.commands'],
    ['command too long', manifestText({ commands: [`A${'B'.repeat(32)}`] }), 'api.manifest.commands'],
  ];
  for (const [label, text, code] of cases) {
    const error = expectManifestCode(() => parsePluginManifest(text), code);
    assert.ok(error.message.length > 0, `${label} must carry a message`);
  }
});

test('unknown keys are named one at a time', () => {
  const error = expectManifestCode(
    () => parsePluginManifest(manifestText({ first: 1, second: 2 })),
    'api.manifest.unknown_key: first',
  );
  assert.match(error.message, /first/);
});
