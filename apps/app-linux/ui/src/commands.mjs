// Command catalog, admission rules and command-line autocomplete.
//
// `SUPPORTED_COMMANDS` mirrors the host's `normalize_cad_command`: only `OPEN`
// and `FIT` reach `cad_command`. Everything else the app names is shown, but
// marked unsupported so the UI can disable it with an explicit reason instead
// of pretending it works.

/** Commands the host admits today (host `normalize_cad_command`). */
export const SUPPORTED_COMMANDS = Object.freeze(['OPEN', 'FIT']);

/** Normalize typed input the same way the host does (trim + upper-case). */
export function normalizeCommandText(text) {
  return String(text ?? '').trim().toUpperCase();
}

export function isSupportedCommand(id) {
  return SUPPORTED_COMMANDS.includes(normalizeCommandText(id));
}

/**
 * Every command name the app exposes in the chrome. `labelKey` is a catalog
 * key; `icon` names an entry in the icon set. Order is the AutoCAD-style
 * listing order (file, draw, modify, view, measure, annotate, classify).
 */
export const COMMAND_CATALOG = Object.freeze([
  { id: 'OPEN', labelKey: 'file.open', icon: 'open', group: 'file' },
  { id: 'FIT', labelKey: 'toolbar.fit', icon: 'fit', group: 'view' },
  { id: 'NEW', labelKey: 'qat.new', icon: 'new', group: 'file' },
  { id: 'SAVE', labelKey: 'qat.save', icon: 'save', group: 'file' },
  { id: 'UNDO', labelKey: 'toolbar.undo', icon: 'undo', group: 'modify' },
  { id: 'REDO', labelKey: 'toolbar.redo', icon: 'redo', group: 'modify' },
  { id: 'LINE', labelKey: 'draw.kind.line', icon: 'line', group: 'draw' },
  { id: 'CIRCLE', labelKey: 'draw.kind.circle', icon: 'circle', group: 'draw' },
  { id: 'ARC', labelKey: 'draw.kind.arc', icon: 'arc', group: 'draw' },
  { id: 'MOVE', labelKey: 'draw.kind.move', icon: 'move', group: 'modify' },
  { id: 'COPY', labelKey: 'draw.kind.copy', icon: 'copy', group: 'modify' },
  { id: 'TRIM', labelKey: 'draw.kind.trim', icon: 'trim', group: 'modify' },
  { id: 'EXTEND', labelKey: 'draw.kind.extend', icon: 'extend', group: 'modify' },
  { id: 'ROTATE', labelKey: 'draw.kind.rotate', icon: 'rotate', group: 'modify' },
  { id: 'SCALE', labelKey: 'draw.kind.scale', icon: 'scale', group: 'modify' },
  { id: 'MIRROR', labelKey: 'draw.kind.mirror', icon: 'mirror', group: 'modify' },
  { id: 'OFFSET', labelKey: 'draw.kind.offset', icon: 'offset', group: 'modify' },
  { id: 'DIST', labelKey: 'measure.kind.distance', icon: 'distance', group: 'measure' },
  { id: 'PLINE', labelKey: 'measure.kind.polyline', icon: 'polyline', group: 'measure' },
  { id: 'ANGLE', labelKey: 'measure.kind.angle', icon: 'angle', group: 'measure' },
  { id: 'AREA', labelKey: 'measure.kind.area', icon: 'area', group: 'measure' },
  { id: 'LAYER', labelKey: 'layers.panel', icon: 'layers', group: 'annotate' },
  { id: 'PROPERTIES', labelKey: 'properties.panel', icon: 'properties', group: 'annotate' },
  { id: 'SELECTALL', labelKey: 'selection.all', icon: 'selectAll', group: 'annotate' },
  { id: 'ZOOMIN', labelKey: 'toolbar.zoom_in', icon: 'zoomIn', group: 'view' },
  { id: 'ZOOMOUT', labelKey: 'toolbar.zoom_out', icon: 'zoomOut', group: 'view' },
  { id: 'PAN', labelKey: 'toolbar.pan', icon: 'pan', group: 'view' },
  { id: 'ORBIT', labelKey: 'view.orbit_label', icon: 'orbit', group: 'view' },
  { id: 'GRID', labelKey: 'overlay.grid', icon: 'grid', group: 'view' },
  { id: 'SNAP', labelKey: 'status.toggle.snap', icon: 'snap', group: 'view' },
  { id: 'ORTHO', labelKey: 'status.toggle.ortho', icon: 'angle', group: 'view' },
  { id: 'OSNAP', labelKey: 'status.toggle.osnap', icon: 'snap', group: 'view' },
]);

const CATALOG_BY_ID = new Map(COMMAND_CATALOG.map(command => [command.id, command]));

export function commandById(id) {
  return CATALOG_BY_ID.get(normalizeCommandText(id)) ?? null;
}

/**
 * Autocomplete over the command catalog.
 *
 * Ordering follows the command-window convention: prefix matches before
 * mid-string matches, then commands used more often in history float up, then
 * alphabetical for stability. `query` is the raw typed text.
 */
export function filterCommandSuggestions(query, { history = [], limit = 8, catalog = COMMAND_CATALOG } = {}) {
  const needle = normalizeCommandText(query);
  const usage = new Map();
  history.forEach((entry, index) => {
    const id = normalizeCommandText(entry);
    usage.set(id, (usage.get(id) ?? 0) + (history.length - index));
  });
  const matches = [];
  for (const command of catalog) {
    const at = needle ? command.id.indexOf(needle) : 0;
    if (needle && at === -1) continue;
    matches.push({
      ...command,
      supported: isSupportedCommand(command.id),
      score: needle ? (at === 0 ? 0 : 1) : 2,
      usage: usage.get(command.id) ?? 0,
    });
  }
  matches.sort((a, b) => a.score - b.score || b.usage - a.usage || a.id.localeCompare(b.id));
  return matches.slice(0, limit).map(({ score, usage: _usage, ...rest }) => rest);
}

/** The suggestion shown highlighted after `query` changes. */
export function defaultSuggestionIndex(suggestions, query) {
  if (!suggestions.length) return -1;
  const needle = normalizeCommandText(query);
  if (!needle) return 0;
  const exact = suggestions.findIndex(suggestion => suggestion.id === needle);
  return exact === -1 ? 0 : exact;
}

/**
 * Step the history cursor. `null` means "the draft line". Up moves to older
 * entries, Down moves back toward the draft.
 */
export function stepHistoryCursor(history, index, direction) {
  if (!history.length) return null;
  const size = history.length;
  if (direction < 0) {
    const base = index === null ? size : index;
    return Math.max(0, base - 1);
  }
  if (index === null) return null;
  const next = index + 1;
  return next >= size ? null : next;
}

/** Text for the current history cursor position. */
export function historyText(history, index, draft) {
  return index === null || index < 0 || index >= history.length ? draft : history[index];
}

/** Push `entry` onto a bounded history (newest last, no adjacent duplicates). */
export function pushHistory(history, entry, limit = 64) {
  const value = String(entry ?? '').trim();
  if (!value) return history;
  const withoutDuplicate = history.at(-1) === value ? history : [...history, value];
  return withoutDuplicate.length > limit ? withoutDuplicate.slice(withoutDuplicate.length - limit) : withoutDuplicate;
}

/**
 * Admission for the command line: what would `cad_command` do with this text?
 * Mirrors `cad_command`/`commandEnabled` without touching IPC.
 */
export function admitCommandText(text, { status = null, pending = false } = {}) {
  const id = normalizeCommandText(text);
  if (!id) return { kind: 'empty', command: id };
  if (!isSupportedCommand(id)) return { kind: 'unsupported', command: id };
  if (!status || pending || status.loading) return { kind: 'unavailable', command: id };
  if (id === 'OPEN' && !status.canOpen) return { kind: 'unavailable', command: id };
  if (id === 'FIT' && !(status.canFit && status.documentLabel !== null && status.entityCount !== null)) {
    return { kind: 'unavailable', command: id };
  }
  return { kind: 'admitted', command: id };
}
