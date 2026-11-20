// The units / recovery rows of the Options control table (ADR-0037 D9/D12).
//
// Split out of `options.ts` so both files stay inside the file-size gate. The
// rows, their closed token set and the one cross-field rule live here; the table
// itself (`OPTIONS_FIELDS`) splices them in at the `files` tab position, so the
// rendering order is still declared in exactly one place.

/** One entry of the staged-settings table (see `options.ts`). */
export type OptionField =
  | (OptionWritableField & { readonly kind: 'bool' | 'color' })
  | (OptionWritableField & {
    readonly kind: 'number';
    readonly min: number;
    readonly max: number;
    readonly unitKey: string;
  })
  | (OptionWritableField & {
    // A fractional number (the wire shape is a JSON real, never a truncated
    // integer). Same facts as `number`; only the parser differs.
    readonly kind: 'real';
    readonly min: number;
    readonly max: number;
    readonly unitKey: string;
  })
  | (OptionWritableField & {
    readonly kind: 'select';
    readonly choices: readonly string[];
    readonly choiceKeyPrefix?: string;
    readonly errorCode?: string;
  })
  // A free-form filesystem path whose **empty** value is the documented "no
  // override" state (the fonts directory row). It is writable, so it carries the
  // write key; the host re-validates it like any other setting.
  | (OptionWritableField & { readonly kind: 'path' })
  | (OptionFieldBase & { readonly kind: 'readonly'; readonly wire: string })
  | (OptionFieldBase & { readonly kind: 'locale' });

/** The facts every control carries. */
interface OptionFieldBase {
  readonly id: string;
  readonly tab: string;
  readonly labelKey: string;
  readonly helpKey: string;
}

/**
 * A control whose draft is a `cad_set_setting` write: the `cad_settings` field
 * it is seeded from (`wire`) and the write key `cad_set_setting` expects (`key`).
 */
interface OptionWritableField extends OptionFieldBase {
  readonly wire: string;
  readonly key: string;
}

/**
 * The drawing/display unit tokens (`"drawing" | "mm" | "cm" | "m" | "in" | "ft"`).
 *
 * The schema's stable lowercase set (`cad_app::viewer_config::UnitToken`),
 * default first. `drawing` is the untyped drawing unit; a typed display unit over
 * it is refused by the host, and [`unitPairError`] mirrors that rule so Apply is
 * blocked instead of sending an illegal pair.
 */
export const UNIT_TOKENS: readonly string[] = Object.freeze(['drawing', 'mm', 'cm', 'm', 'in', 'ft']);

/** The `files` tab's units/recovery rows, in rendering order. */
export const UNIT_FIELDS: readonly OptionField[] = Object.freeze<OptionField[]>([
  {
    // The document's units (ADR-0037 D9). Both rows are the same closed token
    // set; the host refuses a typed display unit over the untyped `drawing`
    // source, and [`unitPairError`] mirrors that one cross-field rule.
    id: 'drawingUnit',
    tab: 'files',
    kind: 'select',
    wire: 'drawingUnit',
    key: 'drawingUnit',
    choices: UNIT_TOKENS,
    choiceKeyPrefix: 'options.unitToken',
    labelKey: 'options.field.drawingUnit',
    helpKey: 'options.help.drawingUnit',
  },
  {
    id: 'displayUnit',
    tab: 'files',
    kind: 'select',
    wire: 'displayUnit',
    key: 'displayUnit',
    choices: UNIT_TOKENS,
    choiceKeyPrefix: 'options.unitToken',
    labelKey: 'options.field.displayUnit',
    helpKey: 'options.help.displayUnit',
  },
  {
    // Decimal places the displayed lengths use (`ui.units.decimalPlaces`).
    // The host's range is `0..=8`; an out-of-range value is refused by name.
    id: 'unitPrecision',
    tab: 'files',
    kind: 'number',
    wire: 'unitPrecision',
    key: 'unitPrecision',
    min: 0,
    max: 8,
    unitKey: 'options.unit.decimal_places',
    labelKey: 'options.field.unitPrecision',
    helpKey: 'options.help.unitPrecision',
  },
  {
    // How often a dirty document rewrites its recovery snapshot
    // (`ui.recovery.intervalSeconds`). `0` turns the snapshot writes off and
    // discards the current one; 3600 is the host's ceiling.
    id: 'recoveryInterval',
    tab: 'files',
    kind: 'number',
    wire: 'recoveryInterval',
    key: 'recoveryInterval',
    min: 0,
    max: 3600,
    unitKey: 'options.unit.seconds',
    labelKey: 'options.field.recoveryInterval',
    helpKey: 'options.help.recoveryInterval',
  },
]);

/**
 * Whether the staged unit pair violates the schema's one cross-field rule.
 *
 * A typed display unit over the untyped `drawing` source would present untyped
 * coordinates as a measured length; the host refuses it by name
 * (`ui.units.displayUnit`). An absent (still unreported) token is not judged: the
 * host's own snapshot is never re-classified as invalid.
 */
export function unitPairError(draft: {
  readonly drawingUnit?: string | boolean | null;
  readonly displayUnit?: string | boolean | null;
}): boolean {
  const drawingUnit = draft.drawingUnit;
  const displayUnit = draft.displayUnit;
  return (
    typeof drawingUnit === 'string' && typeof displayUnit === 'string'
    && UNIT_TOKENS.includes(drawingUnit) && UNIT_TOKENS.includes(displayUnit)
    && drawingUnit === 'drawing' && displayUnit !== 'drawing'
  );
}
