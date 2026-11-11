// The experimental-DWG save confirmation.
//
// SAVE on a `.dwg` source must ask before the host opens the save picker: the
// host would offer a NEW `<stem>-export.dwg` and never overwrite the opened
// drawing, but that fact was previously only visible in the picker/status line.
// These are pure-model and source-contract tests in the style of the other
// selector suites (no React DOM rendering): the decision is real logic, the
// wiring is asserted against `App.tsx` and the dialog source.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DWG_SAVE_COMMANDS,
  dwgExportName,
  dwgSaveConfirmationRequest,
  isDwgSourceName,
  resolveDwgSaveConfirmation,
  sourceExtension,
  sourceStem,
} from '../src/commands/save-confirm.ts';
import { createTranslator, formatMessage } from '../src/i18n.ts';
import { readCatalog, readModule } from './helpers.ts';

test('a .dwg source asks for the confirmation before the picker', async () => {
  // The source identity is the shell's existing `documentLabel`, including a
  // Windows path and a case-insensitive extension.
  for (const label of ['plan.dwg', 'plan.DWG', '/tmp/workshop/plan.dwg', 'C:\\drawings\\plan.Dwg', '  plan.dwg  ']) {
    assert.equal(isDwgSourceName(label), true, label);
  }
  // Not a DWG source: another extension, no extension, a hidden file, a nested
  // extension and a trailing dot all keep the immediate dispatch.
  for (const label of ['plan.dxf', 'plan', '.dwg', 'plan.dwg.bak', 'plan.', null, undefined, '', '  ']) {
    assert.equal(isDwgSourceName(label), false, String(label));
  }
  assert.equal(sourceExtension('/tmp/workshop/plan.DWG'), 'dwg');
  assert.equal(sourceStem('C:\\drawings\\plan.dwg'), 'plan');
  // The default name mirrors `cad_host::policy::default_dwg_export_name`: the
  // host's own suggestion is `<stem>-export.dwg`, never the source name.
  assert.equal(dwgExportName('/tmp/workshop/plan.dwg'), 'plan-export.dwg');
  assert.equal(dwgExportName('archive.tar.dwg'), 'archive.tar-export.dwg');
  assert.equal(dwgExportName(null), 'drawing-export.dwg');

  const request = dwgSaveConfirmationRequest('SAVE', '/tmp/workshop/plan.dwg');
  assert.deepEqual(request, { command: 'SAVE', exportName: 'plan-export.dwg' });
  // Every name the host routes through the one save funnel asks the same
  // question; a different command and a `.dxf` source do not.
  for (const command of DWG_SAVE_COMMANDS) {
    assert.deepEqual(
      dwgSaveConfirmationRequest(command.toLowerCase(), 'plan.dwg'),
      { command, exportName: 'plan-export.dwg' },
      command,
    );
  }
  assert.equal(dwgSaveConfirmationRequest('PLOT', 'plan.dwg'), null);
  assert.equal(dwgSaveConfirmationRequest('SAVE', 'plan.dxf'), null);
  assert.equal(dwgSaveConfirmationRequest('SAVE', null), null);

  // The App asks only after admission and before the one command funnel, so a
  // refused SAVE keeps its own reason and nothing reaches the host before the
  // user answers.
  const app = await readModule('App.tsx');
  const hook = await readModule('components/use-dwg-save-confirm.tsx');
  const gateAt = app.indexOf('confirmDwgSave(text, status, documentLabel, vocabularyRef');
  const queueAt = app.indexOf('await queueCadCommand(invoke, text, status, false');
  assert.ok(gateAt > 0, 'the shell asks the hook before it dispatches');
  assert.ok(queueAt > gateAt, 'the question is asked before `cad_command` is sent');
  assert.ok(app.includes('{dwgSaveDialog}'), 'the hook owns the dialog slot');
  assert.ok(hook.includes('dwgSaveConfirmationRequest(admission.command, documentLabel)'));
  assert.ok(hook.includes('setActive(confirmation)'), 'the dialog state is opened');
  assert.ok(hook.includes('<DwgSaveConfirmDialog'), 'the confirmation is rendered');
});

test('cancel closes the confirmation and dispatches nothing', async () => {
  const request = dwgSaveConfirmationRequest('SAVE', 'plan.dwg');
  assert.notEqual(request, null);
  assert.deepEqual(
    resolveDwgSaveConfirmation(request, 'cancel'),
    { confirmation: null, dispatch: null },
  );
  // Cancel is the initial focus: the safe answer, not the write.
  const dialog = await readModule('components/DwgSaveConfirmDialog.tsx');
  assert.ok(dialog.includes('initial: cancelRef'));
  assert.ok(dialog.includes('onClick={onCancel}'));
  // The hook's cancel path only resolves the pending request; the redispatch
  // closure belongs to the proceed branch and is never reached here.
  const hook = await readModule('components/use-dwg-save-confirm.tsx');
  assert.ok(hook.includes("const cancel = useCallback(() => resolve('cancel')"));
  const resolveAt = hook.indexOf('const resolve =');
  const proceedAt = hook.indexOf('const proceed =');
  const resolveBody = hook.slice(resolveAt, proceedAt);
  assert.ok(resolveBody.includes('pending.current = null;'), 'the one-shot entry is cleared');
  assert.ok(resolveBody.includes('pendingSave.redispatch()'));
  assert.equal(resolveBody.includes('submitCommand'), false, 'cancel never dispatches');
});

test('proceed dispatches the admitted SAVE exactly once', async () => {
  const request = dwgSaveConfirmationRequest('SAVE', 'plan.dwg');
  const first = resolveDwgSaveConfirmation(request, 'proceed');
  assert.deepEqual(first, { confirmation: null, dispatch: 'SAVE' });
  // The confirmation is consumed: a second answer (double click, key repeat)
  // has nothing left to dispatch.
  assert.deepEqual(
    resolveDwgSaveConfirmation(first.confirmation, 'proceed'),
    { confirmation: null, dispatch: null },
  );

  // The hook clears its one-shot entry before it re-dispatches, so the same
  // guarantee holds in the real state machine, and there is exactly one
  // redispatch call. The App's own bypass is the third argument, set only by
  // that closure.
  const hook = await readModule('components/use-dwg-save-confirm.tsx');
  const clearAt = hook.indexOf('pending.current = null;');
  const dispatchAt = hook.indexOf('pendingSave.redispatch()');
  assert.ok(clearAt > 0, 'the pending request is cleared');
  assert.ok(dispatchAt > clearAt, 'the entry is cleared before the dispatch');
  assert.equal(hook.split('pendingSave.redispatch()').length - 1, 1);
  const app = await readModule('App.tsx');
  assert.equal(app.split('void submitCommand(text, options, true)').length - 1, 1);
  assert.ok(app.includes('dwgSaveConfirmed'), 'the proceed path is the one bypass');
});

test('the confirmation copy and both actions exist in both catalogs', async () => {
  const zh = await readCatalog('zh-CN');
  const en = await readCatalog('en');
  const keys = [
    'save.dwg_confirm.title',
    'save.dwg_confirm.body',
    'save.dwg_confirm.proceed',
    'save.dwg_confirm.cancel',
  ];
  for (const key of keys) {
    assert.equal(typeof zh[key], 'string', `zh-CN missing ${key}`);
    assert.equal(typeof en[key], 'string', `en missing ${key}`);
    assert.ok(zh[key].trim().length > 0, key);
    assert.ok(en[key].trim().length > 0, key);
  }
  // One placeholder, identical on both sides, and the rendered Chinese body is
  // the required caution sentence with the host's suggested name.
  const placeholder = /\{([A-Za-z_][A-Za-z0-9_]*)\}/g;
  assert.deepEqual(
    [...zh['save.dwg_confirm.body'].matchAll(placeholder)].map(match => match[1]),
    ['name'],
  );
  assert.deepEqual(
    [...en['save.dwg_confirm.body'].matchAll(placeholder)].map(match => match[1]),
    ['name'],
  );
  const renderedZh = formatMessage(zh['save.dwg_confirm.body'], { name: 'plan-export.dwg' });
  assert.equal(
    renderedZh,
    'DWG 保存为实验性功能：会另存为新文件（默认 plan-export.dwg），不会覆盖当前打开的源图纸；DWG 导出尚不完整，请保留原文件。',
  );
  const t = createTranslator({ 'zh-CN': zh, en }, 'en');
  assert.equal(t('save.dwg_confirm.proceed'), 'Continue save');
  assert.equal(t('save.dwg_confirm.cancel'), 'Cancel');
  assert.equal(
    t('save.dwg_confirm.body', { name: 'plan-export.dwg' }),
    'Saving as DWG is experimental: it writes a new file (default plan-export.dwg) and never overwrites the opened source drawing. DWG export is incomplete, so keep the original file.',
  );

  // The dialog is one accessible alertdialog: both buttons come from the
  // catalog, the body is described, and the focus trap owns Escape/Tab.
  const dialog = await readModule('components/DwgSaveConfirmDialog.tsx');
  assert.ok(dialog.includes('role="alertdialog"'));
  assert.ok(dialog.includes('aria-modal="true"'));
  assert.ok(dialog.includes('aria-labelledby={titleId}'));
  assert.ok(dialog.includes('aria-describedby={bodyId}'));
  for (const key of keys) assert.ok(dialog.includes(`t('${key}'`), key);
  const focus = await readModule('components/confirm-focus.ts');
  assert.ok(focus.includes("event.key === 'Escape'"));
  assert.ok(focus.includes('onCancel()'));
  assert.ok(focus.includes("event.key !== 'Tab'"));
  assert.ok(focus.includes('preventDefault()'));
  // The card is the existing dialog family, not a second visual language.
  const css = await readModule('style.css');
  assert.ok(css.includes('.popup-overlay.dwg-save-confirm-overlay { z-index: 61; }'));
  assert.ok(css.includes('.dwg-save-confirm__message'));
  assert.ok(css.includes('animation: fade var(--t-med) both;'));
});
