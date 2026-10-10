//! Resources drawer state (F10 / U08-adjacent).
//!
//! The drawer shows **real** resource state only. The host fills the summary
//! types from the concrete sources it already owns — `cad-platform`'s
//! `FontLoadReport`, `cad-app`'s `ImportReport` (`HostController::last_import_report`),
//! the proxy decoder's `ProxyOutput::unsupported`, and the open document's
//! external `resource_keys`. The types here mirror those reports field-for-field
//! so the host mapping is a straight copy; the UI crate stays decoupled from the
//! font loader, importer and proxy decoder (no new dependency edge).
//!
//! Absent data is an explicit empty state: a host that pushes nothing gets the
//! catalog's "no resource data" copy, never a fabricated row. Image entities are
//! not modelled anywhere in this build, so the drawer says so rather than
//! listing a fake image table.

use crate::i18n::MessageSource;

/// One localized resource line in the drawer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRowUi {
    /// Section header, set on the first row of a section and empty afterwards so
    /// the shell can group rows without a nested model.
    pub section: String,
    /// Metric name within the section (may be empty for a bare value).
    pub label: String,
    /// Value text, already localized/formatted by the constructor.
    pub value: String,
}

/// Font loading summary, mirroring `cad_platform::fonts::FontLoadReport`.
///
/// Counts are copied as numbers; `failed`/`unresolved`/`default_face` carry the
/// report's own strings (host data, never invented).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontResourceSummary {
    pub catalog_entries: usize,
    pub requested: usize,
    pub planned: usize,
    pub registered: usize,
    pub failed: Vec<String>,
    pub unresolved: Vec<String>,
    pub default_face: Option<String>,
}

/// Import completeness verdict plus its real count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResourceCompleteness {
    Complete,
    Partial(usize),
    Missing(usize),
    #[default]
    Unverified,
}

/// Import summary, mirroring the fields the drawer surfaces from `ImportReport`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportResourceSummary {
    /// Document identity text (host-supplied; may be a file name or handle).
    pub identity: String,
    /// DWG/DXF version string, when the importer reported one.
    pub dwg_version: String,
    pub completeness: ResourceCompleteness,
    /// Number of import diagnostics, or `None` when the report was not measured.
    pub diagnostics: Option<usize>,
    /// Parse/normalise wall-clock time in milliseconds, when measured.
    pub parse_ms: Option<u64>,
}

/// One proxy record the decoder could not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyUnsupportedRow {
    pub record_type: u32,
    /// The decoder's own reason string (host data, shown verbatim).
    pub reason: String,
    /// Payload size in bytes (the drawer never shows the raw payload).
    pub bytes: usize,
}

/// Proxy decode summary, mirroring `cad_proxy::ProxyOutput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyResourceSummary {
    /// Geometry records the decoder did produce.
    pub decoded_records: usize,
    /// Records rejected with evidence; empty means everything decoded.
    pub unsupported: Vec<ProxyUnsupportedRow>,
}

/// External resource references, mirroring `cad_app::Document::resource_keys`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferencesResourceSummary {
    pub keys: Vec<String>,
}

/// The optional sections a host can report; `None` means the host pushed nothing
/// for that source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceSections {
    pub fonts: Option<FontResourceSummary>,
    pub import: Option<ImportResourceSummary>,
    pub proxy: Option<ProxyResourceSummary>,
    pub references: Option<ReferencesResourceSummary>,
    /// Whether this build models image entities. False today: the database has
    /// no image type, so the drawer states the limitation explicitly instead of
    /// rendering an empty list that would read as "no images".
    pub images_modeled: bool,
}

/// Resources drawer snapshot pushed into the shell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourcesPanelState {
    pub rows: Vec<ResourceRowUi>,
    /// Explicit empty-state text shown when the host pushed no source.
    pub empty_label: String,
    /// Number of distinct sources the host reported (0 means nothing pushed).
    pub source_count: usize,
}

impl ResourcesPanelState {
    /// Build the drawer from the host's real resource sections.
    ///
    /// A section is included only when the host supplied it; the image
    /// limitation is stated whenever anything else was pushed, so the user never
    /// mistakes silence for "no images".
    pub fn from_sections(sections: &ResourceSections, messages: &MessageSource) -> Self {
        let mut rows = Vec::new();
        let mut source_count = 0usize;
        if let Some(fonts) = &sections.fonts {
            source_count += 1;
            push_fonts(&mut rows, fonts, messages);
        }
        if let Some(import) = &sections.import {
            source_count += 1;
            push_import(&mut rows, import, messages);
        }
        if let Some(proxy) = &sections.proxy {
            source_count += 1;
            push_proxy(&mut rows, proxy, messages);
        }
        if let Some(references) = &sections.references {
            source_count += 1;
            push_references(&mut rows, references, messages);
        }
        if source_count > 0 && !sections.images_modeled {
            let start = rows.len();
            rows.push(row(
                "",
                "",
                messages.text("resources.images_unsupported", &[]),
            ));
            mark_section(&mut rows, start, messages.text("resources.images", &[]));
        }
        ResourcesPanelState {
            rows,
            empty_label: messages.text("resources.empty", &[]),
            source_count,
        }
    }

    /// Whether the host has pushed any real resource data yet.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

fn row(
    section: impl Into<String>,
    label: impl Into<String>,
    value: impl Into<String>,
) -> ResourceRowUi {
    ResourceRowUi {
        section: section.into(),
        label: label.into(),
        value: value.into(),
    }
}

/// Put `name` on the first row a section appended, leaving later rows blank.
fn mark_section(rows: &mut [ResourceRowUi], start: usize, name: String) {
    if let Some(first) = rows.get_mut(start) {
        first.section = name;
    }
}

fn push_fonts(
    rows: &mut Vec<ResourceRowUi>,
    fonts: &FontResourceSummary,
    messages: &MessageSource,
) {
    let start = rows.len();
    rows.push(row(
        "",
        messages.text("resources.fonts.catalog", &[]),
        fonts.catalog_entries.to_string(),
    ));
    rows.push(row(
        "",
        messages.text("resources.fonts.requested", &[]),
        fonts.requested.to_string(),
    ));
    rows.push(row(
        "",
        messages.text("resources.fonts.planned", &[]),
        fonts.planned.to_string(),
    ));
    rows.push(row(
        "",
        messages.text("resources.fonts.registered", &[]),
        fonts.registered.to_string(),
    ));
    if !fonts.failed.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.fonts.failed", &[]),
            fonts.failed.join(", "),
        ));
    }
    if !fonts.unresolved.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.fonts.unresolved", &[]),
            fonts.unresolved.join(", "),
        ));
    }
    if let Some(face) = &fonts.default_face {
        rows.push(row(
            "",
            messages.text("resources.fonts.default_face", &[]),
            face.clone(),
        ));
    }
    mark_section(rows, start, messages.text("resources.fonts", &[]));
}

fn push_import(
    rows: &mut Vec<ResourceRowUi>,
    import: &ImportResourceSummary,
    messages: &MessageSource,
) {
    let start = rows.len();
    if !import.identity.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.import.identity", &[]),
            import.identity.clone(),
        ));
    }
    if !import.dwg_version.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.import.version", &[]),
            import.dwg_version.clone(),
        ));
    }
    rows.push(row(
        "",
        messages.text("resources.import.completeness", &[]),
        completeness_text(messages, import.completeness),
    ));
    if let Some(diagnostics) = import.diagnostics {
        rows.push(row(
            "",
            messages.text("resources.import.diagnostics", &[]),
            diagnostics.to_string(),
        ));
    }
    if let Some(parse_ms) = import.parse_ms {
        rows.push(row(
            "",
            messages.text("resources.import.parse_ms", &[]),
            parse_ms.to_string(),
        ));
    }
    mark_section(rows, start, messages.text("resources.import", &[]));
}

fn push_proxy(
    rows: &mut Vec<ResourceRowUi>,
    proxy: &ProxyResourceSummary,
    messages: &MessageSource,
) {
    let start = rows.len();
    rows.push(row(
        "",
        messages.text("resources.proxy.decoded", &[]),
        proxy.decoded_records.to_string(),
    ));
    if proxy.unsupported.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.proxy.unsupported", &[]),
            messages.text("resources.proxy.none", &[]),
        ));
    } else {
        rows.push(row(
            "",
            messages.text("resources.proxy.unsupported", &[]),
            proxy.unsupported.len().to_string(),
        ));
        for record in &proxy.unsupported {
            let label = messages.text(
                "resources.proxy.record_type",
                &[("type", &record.record_type.to_string())],
            );
            let value = messages.text(
                "resources.proxy.record",
                &[
                    ("reason", &record.reason),
                    ("bytes", &record.bytes.to_string()),
                ],
            );
            rows.push(row("", label, value));
        }
    }
    mark_section(rows, start, messages.text("resources.proxy", &[]));
}

fn push_references(
    rows: &mut Vec<ResourceRowUi>,
    references: &ReferencesResourceSummary,
    messages: &MessageSource,
) {
    let start = rows.len();
    if references.keys.is_empty() {
        rows.push(row(
            "",
            messages.text("resources.references.keys", &[]),
            messages.text("resources.references.none", &[]),
        ));
    } else {
        rows.push(row(
            "",
            messages.text("resources.references.keys", &[]),
            references.keys.len().to_string(),
        ));
        rows.push(row("", "", references.keys.join(", ")));
    }
    mark_section(rows, start, messages.text("resources.references", &[]));
}

fn completeness_text(messages: &MessageSource, completeness: ResourceCompleteness) -> String {
    match completeness {
        ResourceCompleteness::Complete => messages.text("resources.completeness.complete", &[]),
        ResourceCompleteness::Partial(count) => messages.text(
            "resources.completeness.partial",
            &[("count", &count.to_string())],
        ),
        ResourceCompleteness::Missing(count) => messages.text(
            "resources.completeness.missing",
            &[("count", &count.to_string())],
        ),
        ResourceCompleteness::Unverified => messages.text("resources.completeness.unverified", &[]),
    }
}
