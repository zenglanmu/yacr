// Pure, localized view of the selection for the docked properties palette.
//
// The wire block (`cad_status.selection`, validated by `selection-wire.mjs`) is
// the only input: rows arrive with a stable machine `key`, a `group`, an
// `editable` flag and an editing `kind`. This module turns that into the
// AutoCAD-style inspector model — category sections in a fixed order, one row
// per property, and the editor the row can honestly offer:
//
//   * `select`   — an enum whose options the host delivered (order preserved),
//                  or a boolean's two named states;
//   * `number`   — a numeric field with the key's real range, never a clamp;
//   * `text`     — a text field whose grammar is checked before the write;
//   * `value`    — a plain read-only value, never a disabled input.
//
// A mixed row (one `count > 1` selection whose objects disagree) displays the
// existing `properties.mixed` placeholder. Nothing here invents a value: an
// unknown key/group is shown by its raw machine name, exactly like the label
// switch in `property-labels.mjs`.
import { propertyGroupLabel, propertyRowLabel } from './property-labels.mjs';

/** Stable section order, matching the backend's row order within each group. */
export const PROPERTY_GROUP_ORDER = Object.freeze(['entity', 'geometry', 'text', 'display']);

/**
 * Ranges the host enforces for numeric keys (the client mirror only *rejects*,
 * it never clamps). A numeric key without an entry is still a number field; its
 * grammar is the host's authority and the field sends the typed string verbatim.
 */
export const PROPERTY_NUMBER_RANGES = Object.freeze({
  transparency: Object.freeze({ min: 0, max: 90 }),
});

/** The editor a row can offer, from `editable` + `kind` + delivered options. */
export function propertyEditor(row) {
  if (!row || row.editable !== true) return 'value';
  const options = Array.isArray(row.options) ? row.options : [];
  switch (row.kind) {
    case 'enum':
      return options.length > 0 ? 'select' : 'text';
    case 'number':
      return 'number';
    // A boolean has two named states, so it reads as a two-choice select (the
    // wire string is still `true`/`false`). No boolean key is writable today.
    case 'bool':
      return 'select';
    case 'text':
    default:
      return 'text';
  }
}

/**
 * Group rows by their backend `group`, keeping the backend's row order inside a
 * section. Known groups follow [`PROPERTY_GROUP_ORDER`]; a group the catalog does
 * not know is appended in first-seen order rather than dropped or merged.
 */
export function groupPropertyRows(rows) {
  const grouped = new Map();
  for (const row of rows ?? []) {
    const group = row?.group;
    if (!grouped.has(group)) grouped.set(group, []);
    grouped.get(group).push(row);
  }
  const order = [...PROPERTY_GROUP_ORDER];
  for (const group of grouped.keys()) {
    if (!order.includes(group)) order.push(group);
  }
  return order
    .filter(group => grouped.has(group))
    .map(group => ({ group, rows: grouped.get(group) }));
}

/** One localized row view: labels, display value, editor and range. */
export function propertyRowView(row, t, mixedHelp = null) {
  const label = propertyRowLabel(t, row.key);
  const mixed = row.mixed === true;
  // The placeholder carries the row's own label so a mixed row reads as
  // "Multiple values: Layer" instead of an unexplained dash.
  const mixedText = mixed ? t('properties.mixed', { keys: label }) : null;
  return {
    key: row.key,
    group: row.group,
    label,
    kind: row.kind,
    editable: row.editable === true,
    mixed,
    rawValue: row.value,
    value: mixedText ?? row.value,
    mixedText,
    // Mixed rows are the only rows whose write reaches every selected object;
    // the palette shows this once above the sections and as each editor's title.
    mixedHelp: mixed ? mixedHelp : null,
    options: Array.isArray(row.options) ? row.options.slice() : [],
    editor: propertyEditor(row),
    range: row.kind === 'number' ? PROPERTY_NUMBER_RANGES[row.key] ?? null : null,
  };
}

/**
 * The whole palette view.
 *
 * `empty` is the explicit no-selection state. For `count >= 1` the same grouped
 * rows render; `multi` additionally carries the count line and the
 * apply-to-everything note, so a multi-selection is a comfortable read rather
 * than a summary with the real values hidden.
 */
export function propertiesView(selection, t) {
  if (!selection || selection.empty || selection.count === 0) {
    return {
      kind: 'empty',
      count: 0,
      countLabel: null,
      mixedHelp: null,
      sections: [],
    };
  }
  const multiple = selection.count > 1;
  const mixedHelp = multiple
    ? t('properties.edit.mixed_help', { count: selection.count })
    : null;
  const sections = groupPropertyRows(selection.rows).map(({ group, rows }) => ({
    group,
    label: propertyGroupLabel(t, group),
    rows: rows.map(row => propertyRowView(row, t, mixedHelp)),
  }));
  return {
    kind: multiple ? 'multi' : 'single',
    count: selection.count,
    countLabel: multiple ? t('properties.selected_count', { count: selection.count }) : null,
    mixedHelp,
    sections,
  };
}
