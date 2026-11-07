// Pure formatting for the file-info panel's real performance readings.
//
// These helpers are framework-free on purpose: the panel renders the numbers,
// the catalog owns every word around them. A missing or malformed reading is an
// em dash, never a fabricated zero, and no helper invents a denominator the host
// did not measure.

/** The one "not measured" glyph the panel renders. */
const EM_DASH = '\u2014';

/** Binary byte units, smallest first. */
const BYTE_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'] as const;

/** A real measurement: finite and non-negative (the wire's own rule). */
function isMeasurement(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

/**
 * Human-readable binary byte size.
 *
 * Whole bytes below 1 KiB, whole KiB below 1 MiB, and one decimal from MiB up
 * (`412 MiB`, `1.2 GiB`). A trailing `.0` is dropped so a round value stays
 * round. A missing, negative or non-finite value is `—`.
 */
export function formatBytes(value: number | null | undefined): string {
  if (!isMeasurement(value)) return EM_DASH;
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < BYTE_UNITS.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  const text = scaled.toFixed(unit >= 2 ? 1 : 0).replace(/\.0$/, '');
  return `${text} ${BYTE_UNITS[unit]}`;
}

/** A percentage with one decimal (`7.5%`); `—` when there is no reading. */
export function formatPercent(value: number | null | undefined): string {
  if (!isMeasurement(value)) return EM_DASH;
  return `${value.toFixed(1)}%`;
}

/**
 * "used / total (percent)" for a memory reading.
 *
 * The percentage appears only when both sides are real. A missing side keeps its
 * own em dash, so the panel never borrows a total from anywhere. `0` is a real
 * reading and formats normally.
 */
export function formatMemoryUsage(
  used: number | null | undefined,
  total: number | null | undefined,
): string {
  const usedReal = isMeasurement(used);
  const totalReal = isMeasurement(total);
  if (!usedReal && !totalReal) return EM_DASH;
  const usedText = formatBytes(used);
  const totalText = formatBytes(total);
  if (usedReal && totalReal && total > 0) {
    return `${usedText} / ${totalText} (${formatPercent((used / total) * 100)})`;
  }
  return `${usedText} / ${totalText}`;
}
