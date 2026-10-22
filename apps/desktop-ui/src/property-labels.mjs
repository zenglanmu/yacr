// Catalog labels for the properties palette. The machine keys come from
// `cad-app::selection` (`PropertyRow::new` and `PropertyGroup::for_key`); the
// sets are frozen here so every key has an explicit, reviewed translation. Any
// key the catalog does not know is rendered **as itself** — never a guessed
// label and never dropped.

/**
 * The catalog label for a stable machine property key.
 */
export function propertyRowLabel(t, key) {
  switch (key) {
    case 'id': return t('properties.row.id');
    case 'handle': return t('properties.row.handle');
    case 'type': return t('properties.row.type');
    case 'layer': return t('properties.row.layer');
    case 'space': return t('properties.row.space');
    case 'draw_order': return t('properties.row.draw_order');
    case 'instance': return t('properties.row.instance');
    case 'sub_element': return t('properties.row.sub_element');
    case 'status': return t('properties.row.status');
    case 'start': return t('properties.row.start');
    case 'end': return t('properties.row.end');
    case 'length': return t('properties.row.length');
    case 'vertices': return t('properties.row.vertices');
    case 'closed': return t('properties.row.closed');
    case 'bulges': return t('properties.row.bulges');
    case 'center': return t('properties.row.center');
    case 'normal': return t('properties.row.normal');
    case 'radius': return t('properties.row.radius');
    case 'start_angle': return t('properties.row.start_angle');
    case 'sweep': return t('properties.row.sweep');
    case 'ratio': return t('properties.row.ratio');
    case 'degree': return t('properties.row.degree');
    case 'control_points': return t('properties.row.control_points');
    case 'position': return t('properties.row.position');
    case 'triangles': return t('properties.row.triangles');
    case 'block': return t('properties.row.block');
    case 'text': return t('properties.row.text');
    case 'height': return t('properties.row.height');
    case 'font': return t('properties.row.font');
    case 'shape': return t('properties.row.shape');
    case 'size': return t('properties.row.size');
    case 'opaque_type': return t('properties.row.opaque_type');
    case 'opaque_version': return t('properties.row.opaque_version');
    case 'payload_bytes': return t('properties.row.payload_bytes');
    case 'pixels': return t('properties.row.pixels');
    case 'file': return t('properties.row.file');
    case 'visible': return t('properties.row.visible');
    case 'boundary': return t('properties.row.boundary');
    case 'inverted': return t('properties.row.inverted');
    case 'children': return t('properties.row.children');
    case 'subtype': return t('properties.row.subtype');
    case 'measurement': return t('properties.row.measurement');
    case 'display': return t('properties.row.display');
    case 'loops': return t('properties.row.loops');
    case 'style': return t('properties.row.style');
    case 'fill': return t('properties.row.fill');
    case 'roots': return t('properties.row.roots');
    case 'lines': return t('properties.row.lines');
    case 'color': return t('properties.row.color');
    case 'linetype': return t('properties.row.linetype');
    case 'lineweight': return t('properties.row.lineweight');
    case 'transparency': return t('properties.row.transparency');
    // An unknown machine key is shown by its raw key, never fabricated.
    default: return key;
  }
}

/**
 * The catalog label for a stable property-group key (`entity`, `geometry`, …).
 *
 * Groups become section headings in the next palette lane; an unknown group is
 * shown by its raw key here exactly like an unknown row key.
 */
export function propertyGroupLabel(t, group) {
  switch (group) {
    case 'entity': return t('properties.group.entity');
    case 'geometry': return t('properties.group.geometry');
    case 'text': return t('properties.group.text');
    case 'display': return t('properties.group.display');
    default: return group;
  }
}
