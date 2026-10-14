export function parseCadStatus(value) {
  const nullableString = field => value[field] === null || typeof value[field] === 'string';
  const count = field => Number.isSafeInteger(value[field]) && value[field] >= 0;
  const camera = value?.camera;
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !['documentLabel', 'error', 'backend', 'adapter'].every(nullableString)
    || !['loading', 'canOpen', 'canFit'].every(field => typeof value[field] === 'boolean')
    || !(value.entityCount === null || count('entityCount'))
    || !['importCount', 'frameCount'].every(count)
    || !(camera === null || (camera && Array.isArray(camera.center) && camera.center.length === 2
      && camera.center.every(Number.isFinite) && Number.isFinite(camera.scale) && camera.scale > 0))) {
    throw Error('CAD_STATUS_INVALID');
  }
  return value;
}

export function commandEnabled(status, pending, command) {
  return Boolean(status && !pending && !status.loading
    && (command === 'OPEN' ? status.canOpen : command === 'FIT' ? status.canFit && status.documentLabel !== null && status.entityCount !== null : false));
}

export async function queueCadCommand(invoke, text, status, pending) {
  const command = text.trim().toUpperCase();
  if (!['OPEN', 'FIT'].includes(command)) return { kind: 'unsupported', command };
  if (!commandEnabled(status, pending, command)) return { kind: 'failed', error: 'CAD_COMMAND_UNAVAILABLE' };
  try {
    const response = await invoke('cad_command', { command });
    if (!response || response.accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    // Acceptance is only queue admission, never completion or a database update.
    return { kind: 'accepted', command };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
}

export function handleCommandKey(event, submit) {
  if (event.key !== 'Enter' || event.isComposing || event.repeat) return;
  event.preventDefault();
  void submit();
}

export function startCadPolling(invoke, onStatus, onError, clock = globalThis) {
  let stopped = false;
  let timer;
  let failures = 0;
  async function poll() {
    try {
      const status = parseCadStatus(await invoke('cad_status'));
      if (!stopped) { failures = 0; onStatus(status); }
    } catch (error) {
      if (!stopped) { failures++; onStatus(null); onError(String(error)); }
    }
    if (!stopped) timer = clock.setTimeout(poll, failures ? Math.min(4000, 500 * 2 ** Math.min(failures, 3)) : 250);
  }
  void poll();
  return () => { stopped = true; clock.clearTimeout(timer); };
}
