// The landing manifest's schema, and nothing else.
//
// `https://yacr.pages.dev/downloads.json` is a public file the landing site also
// reads. This build needs exactly one field - `release` - and ignores every other
// key (`assets`, `cdnBase`, ...), so a landing change cannot break the check. An
// HTML error page, a proxy notice, an empty body and a JSON array are all
// failures with a stable reason; none of them is ever coerced into a version.

import { MAX_MANIFEST_BYTES } from './config.ts';

/** The release field's accepted spelling (same shape as `version.ts`). */
const RELEASE_PATTERN = /^v?\d+\.\d+\.\d+$/;

/** A successful read carries the manifest's own `release` string, unnormalised. */
export type ManifestParseResult =
  | { readonly ok: true; readonly release: string }
  | { readonly ok: false; readonly reason: string };

/** The UTF-8 byte length of `text` (the size the fetch cap is defined in). */
export function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

function failure(reason: string): ManifestParseResult {
  return { ok: false, reason };
}

/**
 * Validate one manifest body.
 *
 * The size check runs before parsing, so an oversized body is refused without
 * allocating a JSON tree. Every failure is a `manifest.*` reason and every call
 * returns - this function never throws.
 */
export function parseManifest(text: string): ManifestParseResult {
  if (typeof text !== 'string') return failure('manifest.not_text');
  if (utf8Length(text) > MAX_MANIFEST_BYTES) {
    return failure(`manifest.too_large: ${utf8Length(text)} > ${MAX_MANIFEST_BYTES}`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return failure('manifest.invalid_json');
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return failure('manifest.not_object');
  }
  const release = (parsed as Record<string, unknown>).release;
  if (typeof release !== 'string') return failure('manifest.release_missing');
  if (!RELEASE_PATTERN.test(release)) {
    return failure(`manifest.release_invalid: ${JSON.stringify(release)}`);
  }
  return { ok: true, release };
}
