// The in-app update check's constants, in one place.
//
// The manifest is the landing site's release metadata (`apps/landing/downloads.json`,
// deployed at the same origin path used by the download page). The check asks for
// it at most once per `CHECK_INTERVAL_MS`; a network read is bounded by
// `FETCH_TIMEOUT_MS` and by `MAX_MANIFEST_BYTES`.
//
// `RELEASE_PAGE_URL` is the page the notice's "download" action opens. The shell
// owns the same literal in `crates/cad-desktop-shell/src/release.rs` (the Rust
// lane's `cad_open_release_page`); `test/update-config-contract.test.ts` pins the
// two to the same string so the transport and the fallback never drift apart.

/** The landing manifest: `{ "release": "v0.1.6", ... }`. */
export const RELEASE_MANIFEST_URL = 'https://yacr.pages.dev/downloads.json';

// Must equal RELEASE_PAGE_URL in crates/cad-desktop-shell/src/release.rs
export const RELEASE_PAGE_URL = 'https://yacr.pages.dev/#download';

// The project landing page About shows ("Project site"). It is the same site as
// RELEASE_PAGE_URL without the `#download` deep link: the About row names the
// project, not the download section. The native shell's
// `cad_open_release_page` opens the anchored page, so the About "open" action
// reuses that capability and falls back to copying this URL — see
// `components/AboutDialog.tsx`.
export const LANDING_PAGE_URL = 'https://yacr.pages.dev';

/** One manifest read per day; a throttled startup does not fetch at all. */
export const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

/** A manifest read that has not answered within this budget is abandoned. */
export const FETCH_TIMEOUT_MS = 8000;

/** The largest manifest body this build reads (declared and decoded length). */
export const MAX_MANIFEST_BYTES = 64 * 1024;
