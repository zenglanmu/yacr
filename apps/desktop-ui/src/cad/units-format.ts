// One shared formatter for the document's stored `UNITS` settings.
//
// `cad_status.units` is the document header's own block (`lengthFormat` +
// `lengthPrecision`, angle format/precision/base/direction). The status-bar
// coordinate readout and the command-window echo both call this module, so they
// cannot drift into two different fixed decimal counts. A host that omits the
// block (an older carrier, or a status without a document) falls back to the
// legacy fixed four-decimal readout — never to an invented document setting.
//
// `$INSUNITS` insertion scaling and the host Units dialog are not modelled; this
// module formats the values the document already carries, it never rescales one.

export type LengthFormatToken =
  | 'scientific'
  | 'decimal'
  | 'engineering'
  | 'architectural'
  | 'fractional';

export type AngleFormatToken =
  | 'decimal_degrees'
  | 'degrees_minutes_seconds'
  | 'gradians'
  | 'radians'
  | 'surveyors_units';

/** The stored `cad_status.units` block: exactly the document header values. */
export interface CadUnits {
  readonly lengthFormat: LengthFormatToken;
  readonly lengthPrecision: number;
  readonly angleFormat: AngleFormatToken;
  readonly anglePrecision: number;
  readonly angleBaseDegrees: number;
  readonly angleClockwise: boolean;
}

/** A normalized formatter configuration (the wire block or the legacy default). */
export interface UnitsFormat {
  readonly lengthFormat: LengthFormatToken;
  readonly lengthPrecision: number;
  readonly angleFormat: AngleFormatToken;
  readonly anglePrecision: number;
  readonly angleBaseDegrees: number;
  readonly angleClockwise: boolean;
}

const LENGTH_FORMATS: readonly LengthFormatToken[] = Object.freeze([
  'scientific',
  'decimal',
  'engineering',
  'architectural',
  'fractional',
]);

const ANGLE_FORMATS: readonly AngleFormatToken[] = Object.freeze([
  'decimal_degrees',
  'degrees_minutes_seconds',
  'gradians',
  'radians',
  'surveyors_units',
]);

/** The readout used when the host reports no `units` block. */
export const LEGACY_UNITS_FORMAT: UnitsFormat = Object.freeze({
  lengthFormat: 'decimal',
  lengthPrecision: 4,
  angleFormat: 'decimal_degrees',
  anglePrecision: 4,
  angleBaseDegrees: 0,
  angleClockwise: false,
});

/** The formatter configuration for a wire block, or the legacy default. */
export function unitsFormat(units: CadUnits | null | undefined): UnitsFormat {
  if (!units) return LEGACY_UNITS_FORMAT;
  return {
    lengthFormat: units.lengthFormat,
    lengthPrecision: units.lengthPrecision,
    angleFormat: units.angleFormat,
    anglePrecision: units.anglePrecision,
    angleBaseDegrees: units.angleBaseDegrees,
    angleClockwise: units.angleClockwise,
  };
}

/**
 * Validate a `cad_status.units` value.
 *
 * `undefined`/`null` mean "the host reported no block"; every other shape must
 * be complete and in range, so a malformed block can never become a guessed
 * readout format.
 */
export function parseCadUnits(value: unknown): CadUnits | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== 'object' || Array.isArray(value)) {
    throw Error('CAD_STATUS_INVALID');
  }
  const record = value as Record<string, unknown>;
  const lengthFormat = record.lengthFormat;
  const angleFormat = record.angleFormat;
  if (!isToken(lengthFormat, LENGTH_FORMATS) || !isToken(angleFormat, ANGLE_FORMATS)) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (!isPrecision(record.lengthPrecision) || !isPrecision(record.anglePrecision)) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (typeof record.angleBaseDegrees !== 'number' || !Number.isFinite(record.angleBaseDegrees)) {
    throw Error('CAD_STATUS_INVALID');
  }
  if (typeof record.angleClockwise !== 'boolean') {
    throw Error('CAD_STATUS_INVALID');
  }
  return {
    lengthFormat,
    lengthPrecision: record.lengthPrecision,
    angleFormat,
    anglePrecision: record.anglePrecision,
    angleBaseDegrees: record.angleBaseDegrees,
    angleClockwise: record.angleClockwise,
  };
}

function isToken<T extends string>(value: unknown, allowed: readonly T[]): value is T {
  return typeof value === 'string' && (allowed as readonly string[]).includes(value);
}

function isPrecision(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 8;
}

function precisionOf(value: number): number {
  return Math.min(8, Math.max(0, Math.trunc(value)));
}

/**
 * Format one length (or a coordinate ordinate) in the stored format, or `null`
 * for a non-finite value (which the wire never carries for a real coordinate).
 */
export function formatLength(value: number, units: UnitsFormat): string | null {
  if (!Number.isFinite(value)) return null;
  const precision = precisionOf(units.lengthPrecision);
  const clean = value === 0 ? 0 : value;
  switch (units.lengthFormat) {
    case 'scientific':
      return scientific(clean, precision);
    case 'decimal':
      return fixed(clean, precision);
    case 'engineering':
      return engineering(clean, precision);
    case 'architectural':
      return architectural(clean, precision);
    case 'fractional':
      return fractional(clean, precision);
  }
}

/**
 * Format one angle in degrees.
 *
 * `absolute` applies `ANGBASE`/`ANGDIR` and normalises into `[0, 360)`; a
 * relative measured angle keeps its sign and magnitude. Returns `null` for a
 * non-finite value.
 */
export function formatAngle(
  degrees: number,
  units: UnitsFormat,
  absolute = true,
): string | null {
  if (!Number.isFinite(degrees)) return null;
  const precision = precisionOf(units.anglePrecision);
  let value = degrees;
  if (absolute) {
    value = degrees - units.angleBaseDegrees;
    if (units.angleClockwise) value = -value;
    value = ((value % 360) + 360) % 360;
  }
  switch (units.angleFormat) {
    case 'decimal_degrees':
      return `${fixed(value, precision)}\u00B0`;
    case 'degrees_minutes_seconds':
      return dms(value, precision);
    case 'gradians':
      return `${fixed((value * 10) / 9, precision)}g`;
    case 'radians':
      return `${fixed((value * Math.PI) / 180, precision)}r`;
    case 'surveyors_units':
      return surveyors(value, precision);
  }
}

/** Fixed decimals, with a rounded zero never printed as `-0.0000`. */
function fixed(value: number, precision: number): string {
  const text = value.toFixed(precision);
  return Number(text) === 0 ? (0).toFixed(precision) : text;
}

function scientific(value: number, precision: number): string {
  if (value === 0) return `0.${'0'.repeat(precision)}E+00`;
  const [mantissa, exponentText] = value.toExponential(precision).split('e');
  const exponent = Number(exponentText);
  const sign = exponent < 0 ? '-' : '+';
  const digits = Math.abs(exponent).toString().padStart(2, '0');
  return `${mantissa}E${sign}${digits}`;
}

function engineering(value: number, precision: number): string {
  const sign = value < 0 ? '-' : '';
  const scale = 10 ** precision;
  const total = Math.round(Math.abs(value) * scale);
  const wholeInches = Math.floor(total / scale);
  const fraction = total % scale;
  const feet = Math.floor(wholeInches / 12);
  const inches = wholeInches % 12;
  if (precision === 0) return `${sign}${feet}'-${inches}"`;
  return `${sign}${feet}'-${inches}.${String(fraction).padStart(precision, '0')}"`;
}

function architectural(value: number, precision: number): string {
  const denominator = 2 ** precision;
  const perFoot = 12 * denominator;
  const total = Math.round(Math.abs(value) * denominator);
  const sign = total === 0 ? '' : value < 0 ? '-' : '';
  const feet = Math.floor(total / perFoot);
  const remainder = total % perFoot;
  const inches = Math.floor(remainder / denominator);
  const numerator = remainder % denominator;
  if (numerator === 0) return `${sign}${feet}'-${inches}"`;
  const divisor = gcd(numerator, denominator);
  return `${sign}${feet}'-${inches} ${numerator / divisor}/${denominator / divisor}"`;
}

function fractional(value: number, precision: number): string {
  const denominator = 2 ** precision;
  const total = Math.round(Math.abs(value) * denominator);
  const sign = total === 0 ? '' : value < 0 ? '-' : '';
  const whole = Math.floor(total / denominator);
  const numerator = total % denominator;
  if (numerator === 0) return `${sign}${whole}`;
  const divisor = gcd(numerator, denominator);
  return `${sign}${whole} ${numerator / divisor}/${denominator / divisor}`;
}

function dms(value: number, precision: number): string {
  const sign = value < 0 ? '-' : '';
  const parts = dmsParts(Math.abs(value), precision);
  return `${sign}${parts.degrees}d${parts.minutes}'${parts.seconds}"`;
}

function surveyors(value: number, precision: number): string {
  const angle = ((value % 360) + 360) % 360;
  let prefix: string;
  let inner: number;
  let suffix: string;
  if (angle <= 90) {
    prefix = 'N';
    inner = angle;
    suffix = 'E';
  } else if (angle <= 180) {
    prefix = 'S';
    inner = 180 - angle;
    suffix = 'E';
  } else if (angle <= 270) {
    prefix = 'S';
    inner = angle - 180;
    suffix = 'W';
  } else {
    prefix = 'N';
    inner = 360 - angle;
    suffix = 'W';
  }
  const parts = dmsParts(inner, precision);
  return `${prefix} ${parts.degrees}d${parts.minutes}'${parts.seconds}" ${suffix}`;
}

interface DmsParts {
  readonly degrees: number;
  readonly minutes: number;
  readonly seconds: string;
}

function dmsParts(value: number, precision: number): DmsParts {
  const scale = 10 ** precision;
  const total = Math.round(value * 3600 * scale);
  const secondsPerMinute = 60 * scale;
  const degrees = Math.floor(total / (60 * secondsPerMinute));
  const remainder = total % (60 * secondsPerMinute);
  const minutes = Math.floor(remainder / secondsPerMinute);
  const secondsUnits = remainder % secondsPerMinute;
  const whole = Math.floor(secondsUnits / scale);
  const fraction = secondsUnits % scale;
  const seconds = precision === 0
    ? String(whole)
    : `${whole}.${String(fraction).padStart(precision, '0')}`;
  return { degrees, minutes, seconds };
}

function gcd(left: number, right: number): number {
  let a = left;
  let b = right;
  while (b !== 0) {
    const remainder = a % b;
    a = b;
    b = remainder;
  }
  return Math.max(1, a);
}
