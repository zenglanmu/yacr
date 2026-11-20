//! Save-time collection of the external resources a `.ymodel` can carry
//! (ADR-0040 D9/D10/D13).
//!
//! Three kinds of asset can be packed into an archive so one file is
//! self-contained: shapes/fonts the drawing names, the source bytes of its
//! external references, and the rasters its `IMAGE` entities point at.
//!
//! The split of responsibility is ADR-0040 D13's: `cad-db` only persists bytes
//! that were already decided, and this module is the **I/O** half — it resolves
//! names through the host's effective font/image directories, reads the files,
//! applies the budget and ignores nothing silently. The collected bytes then go
//! through [`cad_app::host::BundledResource`] / `HostController::insert_bundled_resources`
//! into the authoritative database, immediately before the save takes its
//! snapshot.
//!
//! # Rules this module owns
//!
//! * **budget** (D10): one entry ≤ 32 MiB, one save ≤ 64 MiB. An over-budget or
//!   unreadable entry is skipped and reported, never fatal: the rest of the file
//!   still saves.
//! * **keys** (D1): `<sha256(bytes) 前 32 hex>.<ext>`, lowercase extension taken
//!   from the resolved file. The digest is computed **here**, from the bytes that
//!   are about to be stored, because a key that disagrees with its bytes is the
//!   one silent corruption path the store cannot repair.
//! * **names** (D4): the row carries the logical reference verbatim — the font
//!   key / `cad:Image.file` spelling / `ExternalReference.name` the document
//!   already uses. A name already bound to different bytes, or a payload already
//!   bound to a different name, is `resource.name_conflict`, not a rewrite.
//! * **missing is explicit**: a name that resolves to nothing, or whose extension
//!   no kind accepts, is reported with its marker instead of being guessed at or
//!   quietly dropped.
//!
//! Nothing here is persisted. The per-save outcome travels on the save status
//! line (D13: writing it into `tables/compat.json` would land it in the *archive
//! being written* and inside that table's 32-row cap).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_app::host::resources::BundledResource;
use cad_db::DrawingDatabase;
use cad_domain::ymodel::markers;
use cad_domain::BundledKind;

use crate::*;

/// Per-entry budget (ADR-0040 D10): 32 MiB.
pub const RESOURCE_BUDGET_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// Per-save total budget (ADR-0040 D10): 64 MiB.
///
/// Deliberately not 256 MiB: the bytes are resident in the database and the write
/// path keeps more than one temporary copy, so a larger cap would make one save
/// peak above half a gigabyte.
pub const RESOURCE_BUDGET_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// One external resource the document references, as the host could resolve it.
///
/// `resolved` + `size` describe what a save **would** read; `reason` says why it
/// cannot be embedded (unsupported extension, unreadable). The preview command and
/// the save-time collector share this list, so the dialog's "found / not found"
/// groups and the actual collection can never disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCandidate {
    pub kind: BundledKind,
    /// The logical reference the document uses, verbatim.
    pub name: String,
    /// Where the bytes were found (or would be looked for); diagnostics only.
    pub source: Option<String>,
    /// The file a save would read, `None` when nothing resolved.
    pub resolved: Option<PathBuf>,
    /// Size of the resolved file in bytes (`0` when nothing resolved).
    pub size: u64,
    /// The document already carries this resource (ADR-0040 D5): the bytes are
    /// `cad-db` state, so a save keeps them without reading any file.
    pub embedded: bool,
    /// Why this candidate cannot be embedded, when it cannot.
    pub reason: Option<&'static str>,
}

impl ResourceCandidate {
    /// Whether a save could embed this candidate right now.
    ///
    /// "Found" means the bytes are there **and** the resource can be named
    /// (extension in the kind's allowlist). A file this build cannot name is not
    /// claimable as embeddable, so it stays in the dialog's missing group.
    pub fn found(&self) -> bool {
        // An already-embedded resource is available even when the host file is
        // gone: the document *is* the bytes (D5), so the dialog must not offer it
        // in the "not found" group and claim the save will drop it.
        self.embedded || (self.resolved.is_some() && self.reason.is_none())
    }

    fn unresolved(mut self, reason: &'static str) -> Self {
        self.resolved = None;
        self.size = 0;
        self.reason = Some(reason);
        self
    }
}

/// The answer `cad_ymodel_resource_preview` serves (ADR-0040 D17).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourcePreview {
    /// The document's current self-containment flag (D3).
    pub policy: bool,
    /// Every referenced resource, in `font`, `xref`, `image` order.
    pub entries: Vec<ResourceCandidate>,
}

/// One resource this save could not embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSkip {
    pub kind: BundledKind,
    pub name: String,
    pub reason: &'static str,
}

/// What one save did with the document's external resources.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceReport {
    /// Resources the archive carries because of this save (already-present keys
    /// included: they are in the file).
    pub embedded: usize,
    /// Their byte total.
    pub bytes: u64,
    /// Everything that could not be embedded, with its stable marker.
    pub skipped: Vec<ResourceSkip>,
    /// The document write refused (a real technical reason). The save still
    /// wrote its file; this says the file does not carry the resources.
    pub failure: Option<String>,
}

impl ResourceReport {
    /// Whether this save has anything to say beyond "saved".
    pub fn is_empty(&self) -> bool {
        self.embedded == 0 && self.skipped.is_empty() && self.failure.is_none()
    }

    fn skip(&mut self, kind: BundledKind, name: &str, reason: &'static str) {
        self.skipped.push(ResourceSkip {
            kind,
            name: name.to_string(),
            reason,
        });
    }

    /// The user-visible status for one save, composed from the i18n catalog keys
    /// (`save.embed.report` always, `save.embed.report_skipped` when anything was
    /// skipped) plus the machine diagnostic.
    ///
    /// The two sentences are produced with [`catalog_message`] — the same
    /// locale-neutral `key arg=value` rendering every host status uses — so the
    /// catalog owns the prose and this crate carries no copy. `size` is a
    /// human-readable total ([`human_bytes`], the same units the save dialog
    /// shows), because the line is read by a person, not parsed.
    ///
    /// The marker detail ([`Self::diagnostic_line`]) is appended **after** the
    /// sentences, and only when it carries something: a skip, a refused document
    /// write, or a recovery snapshot that could not be discarded (a save warning
    /// that must not disappear just because resources were also embedded). A save
    /// that embedded nothing and skipped nothing produces no text at all, so the
    /// ordinary save status is unchanged.
    pub fn status_text(&self, path: &str, recovery: Option<&str>) -> String {
        let count = self.embedded.to_string();
        let size = human_bytes(self.bytes);
        let mut text = catalog_message(
            "save.embed.report",
            &[("count", count.as_str()), ("size", size.as_str())],
        );
        if !self.skipped.is_empty() {
            let skipped = self.skipped.len().to_string();
            text.push_str("; ");
            text.push_str(&catalog_message(
                "save.embed.report_skipped",
                &[("count", skipped.as_str())],
            ));
        }
        if !self.skipped.is_empty() || self.failure.is_some() || recovery.is_some() {
            text.push_str("; ");
            text.push_str(&self.diagnostic_line(path, recovery));
        }
        text
    }

    /// The machine detail of one save: stable dotted markers and `key=value`
    /// pairs, never localized prose.
    ///
    /// Kept verbatim from the first implementation — operators and tests grep for
    /// `resource.embedded:`/`skip=<name>:<marker>` — but it is a **diagnostic**
    /// now, never the user-facing sentence.
    pub fn diagnostic_line(&self, path: &str, recovery: Option<&str>) -> String {
        let mut line = format!(
            "resource.embedded: path={path} embedded={} bytes={} skipped={}",
            self.embedded,
            self.bytes,
            self.skipped.len()
        );
        for skip in &self.skipped {
            line.push_str(&format!(" skip={}:{}", skip.name, skip.reason));
        }
        if let Some(reason) = recovery {
            line.push_str(&format!(" recovery_error={reason}"));
        }
        if let Some(failure) = &self.failure {
            line.push_str(&format!(" failed={failure}"));
        }
        line
    }
}

/// One catalog key and its arguments, rendered exactly like
/// [`crate::HostCore::message`] does: `key arg=value …`.
///
/// No lookup happens here and none can: the catalog lives in `cad-ui-core`, one
/// layer above this crate, and localization is the UI's job. What travels is the
/// stable key plus its interpolation values, which is what every host status in
/// this crate already does (`save.exported_ymodel`, `command.unsupported`, …).
pub(crate) fn catalog_message(key: &str, values: &[(&str, &str)]) -> String {
    let mut text = key.to_string();
    for (name, value) in values {
        text.push(' ');
        text.push_str(name);
        text.push('=');
        text.push_str(value);
    }
    text
}

/// A byte total as a person reads it (`64 B`, `1.5 KB`, `1.4 MB`).
///
/// Binary units with one decimal below 100, mirroring the save dialog's own
/// formatter (`apps/desktop-ui/src/commands/save-embed.ts::formatByteSize`), so
/// the status and the checklist cannot disagree about "how much".
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let number = if value >= 100.0 {
        format!("{}", value.round() as u64)
    } else {
        // `{:.1}` then drop a trailing `.0`: `1.0 MB` reads better as `1 MB`.
        let formatted = format!("{value:.1}");
        formatted
            .strip_suffix(".0")
            .map(str::to_string)
            .unwrap_or(formatted)
    };
    format!("{number} {}", UNITS[unit])
}

/// The `<32 lowercase hex>` prefix of `bytes`' SHA-256 (ADR-0040 D1).
fn sha256_prefix(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(cad_domain::resource::RESOURCE_KEY_HEX_LEN);
    for byte in &digest[..cad_domain::resource::RESOURCE_KEY_HEX_LEN / 2] {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// The lowercase extension of `path`, when it has one.
fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// A bare file name (no directory component, not a traversal) or `None`.
///
/// A drawing's font key is a name, not a path; anything with a separator would
/// make the archive depend on the host's file system, so it is not a candidate.
fn bare_file_name(name: &str) -> Option<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.split(['/', '\\']).count() != 1
        || trimmed.chars().any(char::is_control)
    {
        return None;
    }
    Some(trimmed)
}

impl HostCore {
    /// The resources the open document references, resolved through the host's
    /// effective directories, plus its self-containment flag.
    ///
    /// Read-only: no file is read into the document, nothing is written and no
    /// revision moves. This is exactly what the preview command serves. A carrier
    /// that installed no native archive refuses by name (`unsupported-runtime:
    /// ymodel`): it has no `.ymodel` file to describe.
    pub fn ymodel_resource_preview(&self) -> CadResult<ResourcePreview> {
        if self.document_archive().is_none() {
            return Err(CadError::Unsupported(
                policy::UNSUPPORTED_YMODEL.to_string(),
            ));
        }
        let Some(drawing) = self.controller.borrow().drawing() else {
            return Ok(ResourcePreview::default());
        };
        Ok(ResourcePreview {
            policy: drawing.embed_resources(),
            entries: self.resolve_candidates(drawing.as_ref()),
        })
    }

    /// Store the self-containment flag on the open document (ADR-0040 D3).
    ///
    /// Returns whether it changed (one revision when it did). This is the
    /// `cad_set_embed_resources` write path; the document, not a host preference,
    /// owns the rule, so `document.json` carries it and a save of an unchanged
    /// document is byte-identical. No archive means no file could carry the flag,
    /// so that carrier refuses instead of storing an unpersistable choice.
    pub fn set_embed_resources(&self, embed: bool) -> CadResult<bool> {
        if self.document_archive().is_none() {
            return Err(CadError::Unsupported(
                policy::UNSUPPORTED_YMODEL.to_string(),
            ));
        }
        self.controller.borrow_mut().set_embed_resources(embed)
    }

    /// Collect and store the document's external resources before a `.ymodel`
    /// save (ADR-0040 D13).
    /// A no-op when the document does not embed resources (D3: byte-identical
    /// archives for every document that never opted in). No failure of this
    /// method may fail the save: every refusal is returned as a report.
    pub(crate) fn collect_ymodel_resources(&self) -> ResourceReport {
        let preview = match self.ymodel_resource_preview() {
            Ok(preview) => preview,
            // Unreachable on the save path (it refuses long before this without an
            // archive), but a report keeps even that a skipped outcome, never a
            // failed save.
            Err(error) => {
                return ResourceReport {
                    failure: Some(error_text(&error)),
                    ..ResourceReport::default()
                }
            }
        };
        if !preview.policy {
            return ResourceReport::default();
        }
        let mut report = ResourceReport::default();
        let mut accepted: Vec<BundledResource> = Vec::new();
        let total = &mut report;
        let mut total_bytes = 0_u64;
        // `(kind, name) → key` for this pass, so two names for one payload and
        // one name for two payloads are both refused here instead of at the
        // database (which would fail the whole insert).
        let mut names: BTreeMap<(BundledKind, String), String> = BTreeMap::new();
        let mut keys: BTreeMap<(BundledKind, String), String> = BTreeMap::new();
        {
            let binding = self.controller.borrow();
            let Some(drawing) = binding.drawing() else {
                return ResourceReport::default();
            };
            for candidate in &preview.entries {
                if candidate.embedded {
                    // Already `cad-db` state: the save keeps it, no file is read
                    // and no revision moves (content addressing, D7).
                    total.embedded += 1;
                    total.bytes += candidate.size;
                    continue;
                }
                if !candidate.found() {
                    total.skip(
                        candidate.kind,
                        &candidate.name,
                        candidate.reason.unwrap_or(markers::RESOURCE_UNAVAILABLE),
                    );
                    continue;
                }
                if candidate.size > RESOURCE_BUDGET_ENTRY_BYTES {
                    total.skip(
                        candidate.kind,
                        &candidate.name,
                        markers::RESOURCE_BUDGET_ENTRY,
                    );
                    continue;
                }
                let Some(path) = candidate.resolved.as_ref() else {
                    continue;
                };
                let bytes: Arc<[u8]> = match std::fs::read(path) {
                    Ok(bytes) if !bytes.is_empty() => Arc::from(bytes.into_boxed_slice()),
                    // A file that vanished or became unreadable between the
                    // preview and the read is a miss, not an empty resource.
                    _ => {
                        total.skip(
                            candidate.kind,
                            &candidate.name,
                            markers::RESOURCE_UNAVAILABLE,
                        );
                        continue;
                    }
                };
                if total_bytes.saturating_add(bytes.len() as u64) > RESOURCE_BUDGET_TOTAL_BYTES {
                    total.skip(
                        candidate.kind,
                        &candidate.name,
                        markers::RESOURCE_BUDGET_TOTAL,
                    );
                    continue;
                }
                let Some(ext) =
                    extension_of(path).filter(|ext| candidate.kind.allows_extension(ext))
                else {
                    total.skip(
                        candidate.kind,
                        &candidate.name,
                        markers::RESOURCE_UNSUPPORTED_KIND,
                    );
                    continue;
                };
                let key = format!("{}.{ext}", sha256_prefix(&bytes));
                if conflicting(&names, &keys, candidate.kind, &candidate.name, &key)
                    || drawing
                        .resource_name(candidate.kind, &key)
                        .is_some_and(|existing| existing != candidate.name)
                    || drawing
                        .resource_key_for_name(candidate.kind, &candidate.name)
                        .is_some_and(|existing| existing != key)
                {
                    total.skip(
                        candidate.kind,
                        &candidate.name,
                        markers::RESOURCE_NAME_CONFLICT,
                    );
                    continue;
                }
                names.insert((candidate.kind, candidate.name.clone()), key.clone());
                keys.insert((candidate.kind, key.clone()), candidate.name.clone());
                total_bytes += bytes.len() as u64;
                accepted.push(BundledResource {
                    kind: candidate.kind,
                    key,
                    name: candidate.name.clone(),
                    source: candidate.source.clone(),
                    bytes,
                });
            }
        }
        if accepted.is_empty() {
            return report;
        }
        // Content addressing makes this idempotent: a second save that accepts the
        // same keys inserts nothing and therefore does not move the revision.
        match self
            .controller
            .borrow_mut()
            .insert_bundled_resources(&accepted)
        {
            Ok(_) => {
                report.embedded = accepted.len();
                report.bytes = total_bytes;
            }
            Err(error) => {
                report.failure = Some(error_text(&error));
            }
        }
        report
    }

    /// Resolve every referenced resource through the host's effective directories.
    fn resolve_candidates(&self, drawing: &DrawingDatabase) -> Vec<ResourceCandidate> {
        let drawing_dir = self
            .drawing_path
            .borrow()
            .as_ref()
            .and_then(|path| path.parent())
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let fonts_dir = self.effective_fonts_dir();
        let images_dir = self.options.images_dir.clone();
        let mut out = Vec::new();

        // Fonts: the drawing names them by bare file name; the host's effective
        // `fonts/` package (or `--fonts-dir`) is the one place they resolve.
        for name in cad_platform::fonts::requested_fonts(drawing) {
            out.push(already_embedded(
                drawing,
                font_candidate(&name, fonts_dir.as_deref()),
            ));
        }

        // External references: only a reference the importer resolved has bytes to
        // copy (D4: a row exists exactly for those).
        for reference in drawing.external_references() {
            out.push(already_embedded(
                drawing,
                xref_candidate(reference, drawing_dir.as_deref()),
            ));
        }

        // Rasters: the same resolution the display uses (drawing directory first,
        // then the explicit `images/` pack); `cad:Image.file` may be an absolute
        // source path, which `resolve_image_file` refuses as unsafe, so the raw
        // spelling is tried as a plain file next (D9).
        let roots: Vec<PathBuf> = drawing_dir
            .iter()
            .cloned()
            .chain(images_dir.iter().cloned())
            .collect();
        for name in cad_platform::images::requested_images(drawing) {
            out.push(already_embedded(drawing, image_candidate(&name, &roots)));
        }
        out
    }
}

/// Mark a candidate the document already carries, so nothing is read for it.
///
/// The document is the authority for its own bytes (D5): once a resource is in
/// `cad-db`, a save keeps it whether or not the host file still exists, and the
/// preview reports it as available with the size the archive will carry. A file
/// copy that happens to still exist is deliberately ignored — the document wins
/// over the host (D9), so the size shown is the size stored.
fn already_embedded(drawing: &DrawingDatabase, candidate: ResourceCandidate) -> ResourceCandidate {
    let Some(key) = drawing.resource_key_for_name(candidate.kind, &candidate.name) else {
        return candidate;
    };
    let Some(bytes) = drawing.resource(candidate.kind, key) else {
        return candidate;
    };
    ResourceCandidate {
        source: drawing
            .resource_source(candidate.kind, key)
            .map(str::to_string)
            .or(candidate.source),
        size: bytes.len() as u64,
        embedded: true,
        reason: None,
        ..candidate
    }
}

/// Whether `(kind, name)` or `(kind, key)` was already claimed differently in
/// this collection pass.
fn conflicting(
    names: &BTreeMap<(BundledKind, String), String>,
    keys: &BTreeMap<(BundledKind, String), String>,
    kind: BundledKind,
    name: &str,
    key: &str,
) -> bool {
    names
        .get(&(kind, name.to_string()))
        .is_some_and(|existing| existing != key)
        || keys
            .get(&(kind, key.to_string()))
            .is_some_and(|existing| existing != name)
}

fn font_candidate(name: &str, fonts_dir: Option<&Path>) -> ResourceCandidate {
    let candidate = ResourceCandidate {
        kind: BundledKind::Font,
        name: name.to_string(),
        source: None,
        resolved: None,
        size: 0,
        embedded: false,
        reason: None,
    };
    let Some(file) = bare_file_name(name) else {
        return candidate.unresolved(markers::RESOURCE_UNAVAILABLE);
    };
    if extension_of(Path::new(file)).is_none_or(|ext| !BundledKind::Font.allows_extension(&ext)) {
        return candidate.unresolved(markers::RESOURCE_UNSUPPORTED_KIND);
    }
    let Some(dir) = fonts_dir else {
        return candidate.unresolved(markers::RESOURCE_UNAVAILABLE);
    };
    readable(candidate, dir.join(file))
}

fn xref_candidate(
    reference: &cad_db::ExternalReference,
    drawing_dir: Option<&Path>,
) -> ResourceCandidate {
    let candidate = ResourceCandidate {
        kind: BundledKind::Xref,
        name: reference.name.clone(),
        source: reference.resolved_path.clone(),
        resolved: None,
        size: 0,
        embedded: false,
        reason: None,
    };
    let Some(resolved) = reference.resolved_path.as_ref() else {
        return candidate.unresolved(markers::RESOURCE_UNAVAILABLE);
    };
    let path = Path::new(resolved);
    let path = match drawing_dir {
        Some(dir) if path.is_relative() => dir.join(path),
        _ => path.to_path_buf(),
    };
    if extension_of(&path).is_none_or(|ext| !BundledKind::Xref.allows_extension(&ext)) {
        return candidate.unresolved(markers::RESOURCE_UNSUPPORTED_KIND);
    }
    readable(candidate, path)
}

fn image_candidate(name: &str, roots: &[PathBuf]) -> ResourceCandidate {
    let candidate = ResourceCandidate {
        kind: BundledKind::Image,
        name: name.to_string(),
        source: None,
        resolved: None,
        size: 0,
        embedded: false,
        reason: None,
    };
    let resolved = resolve_image_path(roots, name);
    let Some(path) = resolved else {
        return candidate.unresolved(markers::RESOURCE_UNAVAILABLE);
    };
    if extension_of(&path).is_none_or(|ext| !BundledKind::Image.allows_extension(&ext)) {
        return candidate.unresolved(markers::RESOURCE_UNSUPPORTED_KIND);
    }
    let candidate = ResourceCandidate {
        source: Some(path.to_string_lossy().to_string()),
        ..candidate
    };
    readable(candidate, path)
}

/// The file an `IMAGE` reference resolves to, if any.
///
/// The display resolution first (safe relative key, case-insensitive scan), then
/// the raw spelling when it is an absolute source path: the document carries the
/// path it was imported with (D9), and copying that file's bytes is the honest
/// read. A browser has no filesystem, so it always answers `None` — the collector
/// simply reports a miss there (and the web carrier refuses the whole command
/// before it ever gets here).
fn resolve_image_path(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(Some(path)) = cad_platform::images::local::resolve_image_file(roots, name) {
        return Some(path);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = roots;
    let path = Path::new(name);
    (path.is_absolute() && path.is_file()).then(|| path.to_path_buf())
}

/// Fill in `resolved`/`size` when `path` is a readable regular file.
///
/// The file is opened (not read to the end): the dialog's "found" means the bytes
/// can be read, and a per-file metadata length is what the size column shows.
fn readable(candidate: ResourceCandidate, path: PathBuf) -> ResourceCandidate {
    match std::fs::metadata(&path) {
        Ok(meta) if meta.is_file() => match std::fs::File::open(&path) {
            Ok(_) => ResourceCandidate {
                source: candidate
                    .source
                    .or_else(|| Some(path.to_string_lossy().to_string())),
                resolved: Some(path),
                size: meta.len(),
                ..candidate
            },
            Err(_) => candidate.unresolved(markers::RESOURCE_UNAVAILABLE),
        },
        _ => candidate.unresolved(markers::RESOURCE_UNAVAILABLE),
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn skip(name: &str, reason: &'static str) -> ResourceSkip {
        ResourceSkip {
            kind: BundledKind::Font,
            name: name.to_string(),
            reason,
        }
    }

    #[test]
    fn byte_totals_read_like_the_dialog_and_never_as_raw_counts() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(64), "64 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        // 1.4 MiB exactly: the value the save dialog shows for the same bytes.
        assert_eq!(human_bytes(1_468_006), "1.4 MB");
        assert_eq!(human_bytes(64 * 1024 * 1024), "64 MB");
        // Three digits drop the decimal instead of rounding to "100.0".
        assert_eq!(human_bytes(1024 * 1024 * 150), "150 MB");
    }

    /// (a) Nothing embedded, nothing skipped: `is_empty()` is what the save
    /// funnel guards on, so that save keeps the ordinary `save.exported_ymodel`
    /// status — no extra text at all (pinned at the host level too).
    #[test]
    fn an_empty_report_is_empty_so_the_ordinary_status_stays() {
        let report = ResourceReport::default();
        assert!(report.is_empty());
        assert_eq!(
            report.diagnostic_line("plan.ymodel", None),
            "resource.embedded: path=plan.ymodel embedded=0 bytes=0 skipped=0"
        );
    }

    /// (b) A clean embed: only the catalog sentence, no machine detail.
    #[test]
    fn a_clean_embed_is_one_catalog_sentence() {
        let report = ResourceReport {
            embedded: 2,
            bytes: 1_468_006,
            ..ResourceReport::default()
        };
        assert!(!report.is_empty());
        assert_eq!(
            report.status_text("plan.ymodel", None),
            "save.embed.report count=2 size=1.4 MB"
        );
    }

    /// (c) A skip: the second catalog sentence plus the marker detail, in that
    /// order (the human line is never replaced by the diagnostic).
    #[test]
    fn a_skipped_resource_adds_the_second_sentence_and_the_detail() {
        let report = ResourceReport {
            embedded: 2,
            bytes: 1_468_006,
            skipped: vec![skip("absent.ttf", markers::RESOURCE_UNAVAILABLE)],
            failure: None,
        };
        assert_eq!(
            report.status_text("plan.ymodel", None),
            "save.embed.report count=2 size=1.4 MB; \
             save.embed.report_skipped count=1; \
             resource.embedded: path=plan.ymodel embedded=2 bytes=1468006 skipped=1 \
             skip=absent.ttf:resource.unavailable"
        );
    }

    /// (d) A refused document write with nothing embedded: the human line still
    /// leads, and the failure is in the detail.
    #[test]
    fn a_failure_reports_the_empty_embed_plus_the_reason() {
        let report = ResourceReport {
            failure: Some("resource.name_conflict: arial.ttf is bound elsewhere".to_string()),
            ..ResourceReport::default()
        };
        assert!(!report.is_empty(), "a failure alone must reach the status");
        assert_eq!(
            report.status_text("plan.ymodel", None),
            "save.embed.report count=0 size=0 B; \
             resource.embedded: path=plan.ymodel embedded=0 bytes=0 skipped=0 \
             failed=resource.name_conflict: arial.ttf is bound elsewhere"
        );
    }

    /// A stale recovery snapshot is a save warning of its own: embedding must not
    /// swallow it, so it rides the detail line even with no skip.
    #[test]
    fn a_stale_recovery_warning_keeps_its_detail_line() {
        let report = ResourceReport {
            embedded: 1,
            bytes: 12,
            ..ResourceReport::default()
        };
        assert_eq!(
            report.status_text("plan.ymodel", Some("recovery.discard_failed")),
            "save.embed.report count=1 size=12 B; \
             resource.embedded: path=plan.ymodel embedded=1 bytes=12 skipped=0 \
             recovery_error=recovery.discard_failed"
        );
    }
}
