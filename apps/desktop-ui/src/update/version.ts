// Version parsing and comparison for the update check.
//
// The manifest and the host both spell a version as `MAJOR.MINOR.PATCH` with an
// optional `v`, and the chrome only ever needs "is the published release newer
// than the running one". Anything that is not exactly three numeric parts is
// rejected (`null`), including a `0.0.0` placeholder - a version that can never
// be a real release must not silently compare as "older than everything".

/** One parsed version: major, minor, patch. */
export type VersionTuple = readonly [number, number, number];

/** The only accepted spelling: optional `v`, three dot-separated decimal parts. */
const VERSION_PATTERN = /^v?(\d+)\.(\d+)\.(\d+)$/;

/**
 * Parse `input` into a comparable tuple, or `null`.
 *
 * Deliberately strict: `1.2`, `1.2.3.4`, `1.2.3-beta`, `v`, whitespace and
 * non-strings are all rejected, and a placeholder `0.0.0` is rejected too. A
 * caller that gets `null` must treat the version as unusable, never as `0.0.0`.
 */
export function parseVersion(input: string): VersionTuple | null {
  if (typeof input !== 'string') return null;
  const match = VERSION_PATTERN.exec(input);
  if (match === null) return null;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3]);
  if (!Number.isSafeInteger(major) || !Number.isSafeInteger(minor) || !Number.isSafeInteger(patch)) {
    return null;
  }
  if (major === 0 && minor === 0 && patch === 0) return null;
  return [major, minor, patch];
}

/** The canonical spelling of a parsed version (`0.1.6`, no `v`). */
export function formatVersion(version: VersionTuple): string {
  return `${version[0]}.${version[1]}.${version[2]}`;
}

/**
 * Compare two parsed versions: `-1` when `left` is older, `1` when newer, `0`
 * when equal. Lexicographic over the three numeric parts, so `0.1.10` is newer
 * than `0.1.9` (a string comparison would get that backwards).
 */
export function compareVersions(left: VersionTuple, right: VersionTuple): number {
  for (let index = 0; index < 3; index += 1) {
    if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
  }
  return 0;
}

/**
 * Whether `latest` is strictly newer than `current`.
 *
 * Both are parsed first; an unusable version on either side is `false`, because
 * "cannot tell" must never become a notice saying an update exists.
 */
export function isNewer(latest: string, current: string): boolean {
  const latestVersion = parseVersion(latest);
  const currentVersion = parseVersion(current);
  if (latestVersion === null || currentVersion === null) return false;
  return compareVersions(latestVersion, currentVersion) > 0;
}
