// Contract tests for the public plugin UI seam.
//
// Two halves: the pure adapter from `@yacr/plugin-api`'s contribution shape to
// the host's `DomainUiPatch`, and the loader lifecycle that installs and removes
// those contributions with the plugin. The host renders no plugin JSX, so these
// tests assert the model the chrome reads (labels, commands, pending sentences),
// not pixels.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import { loadPluginModule } from '../../src/plugin-api/loader.ts';
import { installPluginUi, removePluginUi, uiPatchFor } from '../../src/plugin-api/ui.ts';
import {
  pluginUiIds,
  registerDomainUi,
  resetDomainUi,
  uiLayoutFor,
} from '../../src/domains/ui/registry.ts';
import { itemLabel, panelTitle, tabLabel } from '../../src/ribbon/ribbon-model.ts';

const CONTRACT_URL = new URL('../../../../crates/cad-ui-core/api/plugin-api.json', import.meta.url);
const payload = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));

function status() {
  return { hostProfile: { surfaceMode: 'native-below', documentOwner: 'native' } };
}

function deps(extra = {}) {
  const invoke = async (command) => {
    if (command === 'cad_api_status') return payload;
    throw new Error(`unsupported-runtime: ${command}`);
  };
  return { invoke, status, ...extra };
}

const CONTRIBUTION = {
  id: 'acme.roof-tools',
  domain: 'concept',
  toolbar: [{
    id: 'acme.roof',
    title: 'Roof',
    items: [
      { id: 'ROOF', label: 'Roof', icon: 'line', command: 'LINE' },
      { id: 'DORMER', label: 'Dormer', pendingReason: 'kernel has no dormer op' },
    ],
  }],
  menu: [{ id: 'acme.menu', title: 'Acme', items: [{ id: 'ACME_ROOF', label: 'Roof', command: 'LINE' }] }],
  tray: [{ id: 'acme.panel', title: 'Acme', pendingReason: 'no panel body yet' }],
};

function fresh() {
  resetDomainUi();
  registerDomainUi('concept', {});
}

test('the adapter maps literal text, commands and pending reasons into one patch', () => {
  const patch = uiPatchFor(CONTRIBUTION);
  assert.equal(patch.id, 'acme.roof-tools');
  assert.equal(patch.domain, 'concept');
  const group = patch.toolbar?.[0];
  assert.equal(group?.title, 'Roof');
  assert.equal(group?.items[0]?.label, 'Roof');
  assert.equal(group?.items[0]?.command, 'LINE');
  assert.equal(group?.items[1]?.pendingText, 'kernel has no dormer op');
  assert.equal(patch.menu?.[0]?.label, 'Acme');
  assert.equal(patch.menu?.[0]?.panels[0]?.title, 'Acme');
  assert.equal(patch.tray?.[0]?.title, 'Acme');
  assert.equal(patch.tray?.[0]?.kind, 'pending');
  assert.equal(patch.tray?.[0]?.pendingText, 'no panel body yet');
});

test('installing a contribution extends the concept layout and removing it restores it', () => {
  fresh();
  installPluginUi(CONTRIBUTION);
  const layout = uiLayoutFor('concept');
  const group = layout.toolbar?.find(entry => entry.id === 'acme.roof');
  assert.ok(group, 'the plugin group is in the merged layout');
  assert.equal(group.items.find(item => item.id === 'ROOF')?.label, 'Roof');
  assert.equal(group.items.find(item => item.id === 'DORMER')?.pendingText, 'kernel has no dormer op');
  assert.deepEqual(pluginUiIds(), ['acme.roof-tools']);

  assert.equal(removePluginUi('acme.roof-tools'), true);
  assert.equal(pluginUiIds().length, 0);
  assert.notEqual(uiLayoutFor('concept').toolbar?.some(entry => entry.id === 'acme.roof'), true);
});

test('a loaded plugin installs its UI and deactivate removes it', async () => {
  fresh();
  const module = {
    default: {
      id: 'acme-plugin',
      apiVersion: 1,
      activate: (ctx) => {
        ctx.api.ui.register(CONTRIBUTION);
      },
    },
  };
  const active = await loadPluginModule({ module }, deps());
  assert.deepEqual(pluginUiIds(), ['acme.roof-tools']);
  assert.ok(uiLayoutFor('concept').toolbar?.some(entry => entry.id === 'acme.roof'));

  await active.deactivate();
  assert.deepEqual(pluginUiIds(), []);
  assert.notEqual(uiLayoutFor('concept').toolbar?.some(entry => entry.id === 'acme.roof'), true);
});

test('an activate failure after registering UI leaves no contribution behind', async () => {
  fresh();
  const module = {
    default: {
      id: 'acme-plugin',
      apiVersion: 1,
      activate: (ctx) => {
        ctx.api.ui.register(CONTRIBUTION);
        throw new Error('activate failed');
      },
    },
  };
  await assert.rejects(
    () => loadPluginModule({ module }, deps()),
    (error) => error?.code === 'api.plugin.load_failed: activate',
  );
  assert.deepEqual(pluginUiIds(), []);
  assert.notEqual(uiLayoutFor('concept').toolbar?.some(entry => entry.id === 'acme.roof'), true);
});

test('an unknown host domain is refused by name', () => {
  let caught = null;
  try {
    uiPatchFor({ ...CONTRIBUTION, domain: 'not-a-domain' });
  } catch (error) {
    caught = error;
  }
  assert.equal(caught?.code, 'api.invalid_argument: ui.domain');
});

test('literal text wins over the catalog key', () => {
  const t = (key) => `catalog:${key}`;
  assert.equal(itemLabel(t, { label: 'Roof', labelKey: 'catalog.key' }), 'Roof');
  assert.equal(itemLabel(t, { labelKey: 'catalog.key' }), 'catalog:catalog.key');
  assert.equal(panelTitle(t, { title: 'Roof', titleKey: 'catalog.key' }), 'Roof');
  assert.equal(tabLabel(t, { label: 'Acme', labelKey: 'catalog.key' }), 'Acme');
});
