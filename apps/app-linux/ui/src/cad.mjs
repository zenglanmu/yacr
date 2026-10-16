// Phase keys the backend reports (`cad_app::import_phase_key`). A phase outside
// this set is a contract violation, not a runtime surprise.
const IMPORT_PHASES = new Set(['reading', 'parsing', 'tables', 'entities', 'resolving', 'finishing']);

function parseImportProgress(value) {
  const count = field => Number.isSafeInteger(value[field]) && value[field] >= 0;
  const nullableCount = field => value[field] === null || count(field);
  if (value === null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || !(value.phase === null || IMPORT_PHASES.has(value.phase))
    || !count('entitiesDone')
    || !nullableCount('entitiesTotal')
    || !nullableCount('bytes')
    || typeof value.cancellable !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return value;
}

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
  // Live import progress is optional (`null` when idle) but, when present, must
  // be well-formed; a malformed block is never rendered as a guessed bar.
  parseImportProgress(value.importProgress);
  return value;
}

/** The catalog label for a locale-independent import phase key. */
function importPhaseLabel(t, phase) {
  switch (phase) {
    case 'reading': return t('import.phase.reading');
    case 'parsing': return t('import.phase.parsing');
    case 'tables': return t('import.phase.tables');
    case 'entities': return t('import.phase.entities');
    case 'resolving': return t('import.phase.resolving');
    case 'finishing': return t('import.phase.finishing');
    default: return t('import.phase.unknown');
  }
}

/**
 * A localised view of real import progress for the chrome.
 *
 * `ratio` is `null` when the importer never exposed a total (indeterminate), so
 * the bar animates instead of pretending a percentage; a determinate `ratio` is
 * the real `done / total`, clamped and never extrapolated to 100% before the
 * document is actually published. Returns `null` when there is no live import.
 */
export function importProgressView(progress, t) {
  if (!progress) return null;
  const determinate = progress.entitiesTotal !== null;
  const ratio = determinate
    ? (progress.entitiesTotal > 0 ? Math.min(1, progress.entitiesDone / progress.entitiesTotal) : 1)
    : null;
  const count = determinate
    ? t('import.progress.count', { done: progress.entitiesDone, total: progress.entitiesTotal })
    : t('import.progress.indeterminate', { done: progress.entitiesDone });
  const detail = progress.bytes === null
    ? count
    : `${count}${t('import.detail.separator')}${t('import.progress.bytes', { bytes: progress.bytes })}`;
  return {
    phase: importPhaseLabel(t, progress.phase),
    count,
    detail,
    ratio,
    determinate,
    cancellable: progress.cancellable,
  };
}

/**
 * Ask the host to cancel the in-flight open (`CANCEL`).
 *
 * Only offered while the host reports the import is still cancellable; refusal
 * (already terminal or probe mode) is explicit, never a silent success. A
 * `{kind:'accepted'}` result is queue admission, not a completed cancel — the
 * real outcome surfaces through the next `cad_status`.
 */
export async function requestImportCancel(invoke, status) {
  if (!status?.importProgress?.cancellable) return { kind: 'unavailable' };
  try {
    const response = await invoke('cad_command', { command: 'CANCEL' });
    if (!response || response.accepted !== true) throw Error('CAD_COMMAND_NOT_ACCEPTED');
    return { kind: 'accepted' };
  } catch (error) { return { kind: 'failed', error: String(error) }; }
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
