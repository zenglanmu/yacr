// Contract tests for the editor prompts on the adapter side.
//
// A prompt completes only when the user answers, so it travels the submit/poll
// lane with its own (long) timeout, and `writeMessage` is client-local: the WebView
// message store is the default sink. Both are asserted here; the read lane and the
// live-session operations keep their own suite.
//
// These tests are `.ts` by filename only: they run under Node's type stripping.

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

import { createCadHostBridge } from '../../src/plugin-api/bridge.ts';
import { pluginMessages, resetPluginMessages } from '../../src/plugin-api/messages.ts';
import { openPluginApi } from '../../src/plugin-api/session.ts';

const CONTRACT_URL = new URL('../../../../crates/cad-ui-core/api/plugin-api.json', import.meta.url);
const payload = JSON.parse(await readFile(CONTRACT_URL, 'utf8'));

/** A minimal `cad_status` stand-in: only `hostProfile` is read by the adapter. */
function statusWith(surfaceMode, documentOwner) {
  return { hostProfile: { surfaceMode, documentOwner } };
}

/** An `invoke` double that records every command and answers `answer`. */
function countingInvoke(answer) {
  const commands = [];
  const invoke = async (command, request) => {
    commands.push(command);
    return typeof answer === 'function' ? answer(command, request) : answer;
  };
  return { invoke, commands };
}

/** The stable code of a rejected `CadApiError`. */
function codeOf(error) {
  return error instanceof Error ? error.code : undefined;
}

test('a prompt submits on the mutating lane and waits out empty polls', async () => {
  let submitted = null;
  let polls = 0;
  const { invoke } = countingInvoke((command, request) => {
    if (command === 'cad_api_status') return payload;
    if (command === 'cad_api_submit') {
      submitted = request;
      return { accepted: true };
    }
    if (command === 'cad_api_poll') {
      polls += 1;
      if (polls < 3) return { results: [] };
      return {
        results: [
          { requestId: submitted.requestId, ok: true, value: { status: 'Ok', value: 'hi' } },
        ],
      };
    }
    return { ok: false, error: `unsupported-runtime: cad_api.${command}` };
  });
  const session = await openPluginApi({
    invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 0,
  });

  const answer = await session.api.editor.getString({ message: 'name?' });
  assert.deepEqual(answer, { status: 'Ok', value: 'hi' });
  assert.equal(polls, 3, 'empty polls are retried under the long prompt budget');
  assert.equal(submitted.namespace, 'EditorInput');
  assert.equal(submitted.operation, 'editor.getString');
  assert.deepEqual(submitted.parameters, { message: 'name?' });
});

test('a prompt reads its own budget; a machine operation keeps the five-second one', async () => {
  function neverAnswers() {
    const polls = [];
    const host = countingInvoke((command) => {
      if (command === 'cad_api_status') return payload;
      if (command === 'cad_api_submit') return { accepted: true };
      polls.push(command);
      return { results: [] };
    });
    return { invoke: host.invoke, polls };
  }

  // A tiny injected prompt budget with a delay that crosses it: the prompt stops
  // at its own deadline after the second poll.
  const prompt = neverAnswers();
  const promptBridge = createCadHostBridge({
    invoke: prompt.invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 20,
    maxPolls: 10,
    promptTimeoutMs: 5,
  });
  await assert.rejects(
    promptBridge.mutate({
      namespace: 'EditorInput',
      operation: 'editor.getInteger',
      parameters: {},
    }),
    (error) => {
      assert.match(codeOf(error), /^api\.timeout: apir-/u);
      return true;
    },
  );
  assert.equal(prompt.polls.length, 2, 'the prompt honoured promptTimeoutMs');

  // The same injected budget does not apply to a machine operation: it runs to its
  // poll budget under the five-second deadline.
  const machine = neverAnswers();
  const machineBridge = createCadHostBridge({
    invoke: machine.invoke,
    status: () => statusWith('native-below', 'native'),
    pollDelayMs: 20,
    maxPolls: 10,
    promptTimeoutMs: 5,
  });
  await assert.rejects(
    machineBridge.mutate({
      namespace: 'DatabaseServices',
      operation: 'transaction.commit',
      parameters: {},
    }),
    (error) => {
      assert.match(codeOf(error), /^api\.timeout: apir-/u);
      return true;
    },
  );
  assert.equal(machine.polls.length, 10, 'the machine timeout is not the prompt timeout');
});

test('the WebView message store is the default writeMessage sink', async () => {
  resetPluginMessages();
  const { invoke } = countingInvoke(payload);
  const session = await openPluginApi({ invoke, status: () => statusWith('native-below', 'native') });
  await session.api.editor.writeMessage('hello from the plugin');
  assert.deepEqual(pluginMessages(), ['hello from the plugin']);

  // An injected sink replaces the store rather than adding to it.
  const custom = [];
  const injected = await openPluginApi({
    invoke,
    status: () => statusWith('native-below', 'native'),
    onMessage: (text) => custom.push(text),
  });
  await injected.api.editor.writeMessage('elsewhere');
  assert.deepEqual(custom, ['elsewhere']);
  assert.deepEqual(pluginMessages(), ['hello from the plugin']);
});

