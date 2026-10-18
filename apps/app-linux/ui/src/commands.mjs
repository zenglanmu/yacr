// Command catalog, admission rules and command-line autocomplete.
//
// Admission is **not** a hand-written mirror any more: the exact set of command
// names the Rust router accepts is loaded once at runtime from the `cad_commands`
// IPC command (backed by `cad_ui_core::session::admissible_names`, the single
// source of truth). Until that answer arrives the catalog is empty, so every
// command is explicitly unsupported — never "everything allowed by default".
//
// On top of the router's vocabulary the chrome keeps an explicit **exclusion**
// list: names the router would accept but this React host cannot service yet
// (they need a native file-dialog/printer owner the chrome does not have). The
// exclusion is deliberate and named, not a silent omission — the buttons stay
// visible but disabled with the existing `command.unsupported` reason.

/** Normalize typed input the same way the host does (trim + upper-case). */
export function normalizeCommandText(text) {
  return String(text ?? '').trim().toUpperCase();
}

/**
 * The loaded host vocabulary, uppercased and deduped. Empty until
 * [`setCommandCatalog`] receives the `cad_commands` answer.
 */
let loadedCatalog = Object.freeze([]);

/** Replace the admission catalog with the names the host reported. */
export function setCommandCatalog(names) {
  const seen = new Set();
  for (const name of names ?? []) {
    const id = normalizeCommandText(name);
    if (id) seen.add(id);
  }
  loadedCatalog = Object.freeze([...seen]);
  return loadedCatalog;
}

/** The current admission catalog (empty until the host has answered). */
export function commandCatalog() {
  return loadedCatalog;
}

/**
 * Commands the Rust router accepts but the React host cannot service yet: they
 * are owned by native dialogs/printing that the chrome has no implementation
 * for, so their buttons stay disabled with `command.unsupported`. This is an
 * explicit exclusion list — sourced from the host vocabulary, not a claim that
 * the names are unknown to the router.
 */
export const HOST_EXCLUDED_COMMANDS = Object.freeze([
  'NEW', 'SAVE', 'SAVEAS', 'QSAVE', 'PLOT', 'EXPORT', 'PRINT',
]);

/** The names kept disabled on top of the host vocabulary (for tests/UI copy). */
export function excludedCommandNames() {
  return [...HOST_EXCLUDED_COMMANDS];
}

export function isSupportedCommand(id) {
  const name = normalizeCommandText(id);
  if (!name || HOST_EXCLUDED_COMMANDS.includes(name)) return false;
  return loadedCatalog.includes(name);
}

/**
 * Catalog key for a host tool selector (`draw.line` -> `tool.draw.line`), or
 * `null` when there is no selector so the caller renders no name.
 */
export function toolLabelKey(toolKey) {
  const key = String(toolKey ?? '').trim().toLowerCase();
  return key ? `tool.${key}` : null;
}

/**
 * The CAD command a global key press should submit while an interactive tool is
 * active, or `null` when the press is not ours.
 *
 * `Enter` confirms and `Escape` cancels the active tool, but only when a tool is
 * really active and the key was not typed into a field (`typing`), was not an IME
 * composition, and was not an auto-repeat. A missing tool returns `null` for
 * every key, so Escape keeps its panel-closing behaviour when no tool is running.
 */
export function toolCommandForKey(key, {
  toolActive = false,
  typing = false,
  composing = false,
  repeat = false,
} = {}) {
  if (!toolActive || typing || composing || repeat) return null;
  if (key === 'Enter') return 'CONFIRM';
  if (key === 'Escape') return 'CANCEL';
  return null;
}

/**
 * Every command name the app exposes in the chrome, in AutoCAD-style listing
 * order (file, draw, modify, view, measure, annotate, classify). `labelKey` is a
 * catalog key; `icon` names an entry in the icon set.
 *
 * This is a **display** table only: whether an entry is enabled comes from the
 * loaded host vocabulary ([`isSupportedCommand`]), so a command missing here can
 * still be typed and admitted when the router knows it.
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
 * Uses the vocabulary loaded from the host ([`isSupportedCommand`]) plus the
 * capability gates, without touching IPC.
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
