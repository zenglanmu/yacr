//! Structured, object-level diagnostics (spec v2.0 §11.4, §19).
//!
//! The audit (F11, U08) found that consumers took only the *first* diagnostic
//! and that representation gaps were never aggregated, so a document with a
//! HATCH and a TEXT entity could still be reported as complete once one reason
//! was surfaced. This module keeps *every* reason, attaches each one to the
//! object it came from, and combines per-object and document-level
//! completeness without dropping the weaker verdicts.
//!
//! Reasons carry a stable machine code plus typed parameters. Text is never
//! baked into the code: localization happens in the UI layer (N01), and the
//! core stays locale-independent.

use cad_domain::{Completeness, ObjectId};

/// Stable, machine-readable diagnostic codes.
///
/// These are part of the diagnostics schema and must not be renamed without a
/// schema migration: CLI output and UI localization key off them.
pub mod codes {
    /// A representation fragment could not be built for an object.
    pub const REPRESENTATION_UNAVAILABLE: &str = "representation.unavailable";
    /// A representation fragment was built but is not exact.
    pub const REPRESENTATION_APPROXIMATE: &str = "representation.approximate";
    /// Semantics were read but producing output is not implemented.
    pub const REPRESENTATION_NOT_IMPLEMENTED: &str = "representation.not_implemented";
    /// A resource an object depends on is not available.
    pub const RESOURCE_MISSING: &str = "resource.missing";
    /// A resource exceeds the configured budget for its category.
    pub const RESOURCE_OVER_BUDGET: &str = "resource.over_budget";
    /// A resource reference nested deeper than the configured limit.
    pub const RESOURCE_RECURSION_LIMIT: &str = "resource.recursion_limit";
    /// A referenced font name resolved to no catalog entry.
    pub const FONT_UNRESOLVED: &str = "resource.font_unresolved";
    /// A referenced font resolved but its technology is not supported.
    pub const FONT_UNSUPPORTED: &str = "resource.font_unsupported";
    /// The importer read an entity type it could not model.
    pub const IMPORT_UNKNOWN_ENTITY: &str = "import.unknown_entity";
    /// A proxy record could not be decoded.
    pub const PROXY_UNDECODED: &str = "proxy.undecoded";
    /// A frame's vertex/triangle submission crossed its configured budget.
    pub const RENDER_FRAME_OVER_BUDGET: &str = "render.frame_over_budget";
    /// The GPU device was lost or the backend failed; derived resources are gone.
    pub const RENDER_DEVICE_LOST: &str = "render.device_lost";
}

/// How severe a single reason is.
///
/// Ordering is meaningful: `Info < Warning < Error`. `combine` keeps the most
/// severe so no consumer can downgrade a missing object to a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// A typed, locale-independent parameter attached to a reason.
///
/// Parameters are what a UI localizer substitutes into the message template
/// for `DiagnosticReason::code`; they never contain localized prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticParameter {
    /// An object the reason refers to (also used for `object` itself).
    Object(ObjectId),
    /// A logical resource key (already sanitized by `cad-resources`).
    Key(String),
    /// A machine identifier such as an entity type class name.
    Identifier(String),
    /// A count, for example the number of unresolved fonts.
    Count(u64),
    /// A size in bytes.
    Bytes(u64),
    /// A configured limit that was exceeded.
    Limit(u64),
    /// A recursion depth that was reached.
    Depth(u64),
}

impl DiagnosticParameter {
    fn to_json(&self) -> serde_json::Value {
        match self {
            DiagnosticParameter::Object(id) => serde_json::json!({
                "kind": "object",
                "value": id.0.to_string(),
            }),
            DiagnosticParameter::Key(key) => serde_json::json!({
                "kind": "key",
                "value": key,
            }),
            DiagnosticParameter::Identifier(identifier) => serde_json::json!({
                "kind": "identifier",
                "value": identifier,
            }),
            DiagnosticParameter::Count(count) => serde_json::json!({
                "kind": "count",
                "value": count,
            }),
            DiagnosticParameter::Bytes(bytes) => serde_json::json!({
                "kind": "bytes",
                "value": bytes,
            }),
            DiagnosticParameter::Limit(limit) => serde_json::json!({
                "kind": "limit",
                "value": limit,
            }),
            DiagnosticParameter::Depth(depth) => serde_json::json!({
                "kind": "depth",
                "value": depth,
            }),
        }
    }
}

/// One reason an object (or the document) is not fully supported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReason {
    /// Stable code from [`codes`].
    pub code: String,
    pub severity: Severity,
    /// Typed parameters, in the order the code's template expects them.
    pub parameters: Vec<DiagnosticParameter>,
    /// Completeness verdict this reason implies for its object.
    pub completeness: Completeness,
}

impl DiagnosticReason {
    pub fn new(
        code: impl Into<String>,
        severity: Severity,
        completeness: Completeness,
        parameters: Vec<DiagnosticParameter>,
    ) -> Self {
        DiagnosticReason {
            code: code.into(),
            severity,
            parameters,
            completeness,
        }
    }

    /// A `Missing` reason, e.g. a representation that could not be built.
    pub fn missing(
        code: impl Into<String>,
        parameters: Vec<DiagnosticParameter>,
    ) -> DiagnosticReason {
        DiagnosticReason {
            code: code.into(),
            severity: Severity::Error,
            parameters,
            completeness: Completeness::Missing(Vec::new()),
        }
    }

    /// A `Partial` reason, e.g. an approximation of an exact primitive.
    pub fn partial(
        code: impl Into<String>,
        parameters: Vec<DiagnosticParameter>,
    ) -> DiagnosticReason {
        DiagnosticReason {
            code: code.into(),
            severity: Severity::Warning,
            parameters,
            completeness: Completeness::Partial(Vec::new()),
        }
    }

    /// An `Unverified` reason, e.g. reading succeeded but is untested.
    pub fn unverified(
        code: impl Into<String>,
        parameters: Vec<DiagnosticParameter>,
    ) -> DiagnosticReason {
        DiagnosticReason {
            code: code.into(),
            severity: Severity::Warning,
            parameters,
            completeness: Completeness::Unverified,
        }
    }

    /// The completeness verdict implied by a set of reasons (most severe wins).
    pub fn combine_all(reasons: &[DiagnosticReason]) -> Completeness {
        reasons.iter().fold(Completeness::Complete, |acc, reason| {
            acc.combine(reason.completeness.clone())
        })
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "severity": self.severity.as_str(),
            "parameters": self
                .parameters
                .iter()
                .map(DiagnosticParameter::to_json)
                .collect::<Vec<_>>(),
        })
    }
}

/// Every reason that applies to one object.
///
/// Unlike a `Vec<Diagnostic>` consumed first-match-only, this keeps all reasons
/// and derives the object's completeness from all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectDiagnostics {
    pub object: ObjectId,
    pub reasons: Vec<DiagnosticReason>,
}

impl ObjectDiagnostics {
    pub fn new(object: ObjectId) -> Self {
        ObjectDiagnostics {
            object,
            reasons: Vec::new(),
        }
    }

    pub fn push(&mut self, reason: DiagnosticReason) {
        self.reasons.push(reason);
    }

    pub fn completeness(&self) -> Completeness {
        DiagnosticReason::combine_all(&self.reasons)
    }

    /// The worst severity across every reason (defaults to `Info`).
    pub fn severity(&self) -> Severity {
        self.reasons
            .iter()
            .map(|reason| reason.severity)
            .max()
            .unwrap_or(Severity::Info)
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.completeness(), Completeness::Complete)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "object": self.object.0.to_string(),
            "severity": self.severity().as_str(),
            "reasons": self
                .reasons
                .iter()
                .map(DiagnosticReason::to_json)
                .collect::<Vec<_>>(),
        })
    }
}

/// Document-level summary that distinguishes complete from partial/missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsSummary {
    pub completeness: Completeness,
    pub objects: usize,
    pub complete_objects: usize,
    pub partial_objects: usize,
    pub missing_objects: usize,
    pub unverified_objects: usize,
    pub reasons: usize,
    /// True only when every object is `Complete` and there are no reasons.
    pub complete: bool,
}

/// Aggregates all reasons instead of taking the first one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsModel {
    /// Reasons grouped by the object they came from.
    pub objects: Vec<ObjectDiagnostics>,
    /// Document-level reasons not tied to a specific object.
    pub document: Vec<DiagnosticReason>,
}

impl DiagnosticsModel {
    pub fn new() -> Self {
        DiagnosticsModel::default()
    }

    /// Add a reason for an object, creating its entry on first use.
    pub fn add(&mut self, object: ObjectId, reason: DiagnosticReason) {
        match self.objects.iter_mut().find(|entry| entry.object == object) {
            Some(entry) => entry.push(reason),
            None => {
                let mut entry = ObjectDiagnostics::new(object);
                entry.push(reason);
                self.objects.push(entry);
            }
        }
    }

    /// Add a document-level reason (for example an over-budget resource total).
    pub fn add_document(&mut self, reason: DiagnosticReason) {
        self.document.push(reason);
    }

    pub fn object(&self, object: ObjectId) -> Option<&ObjectDiagnostics> {
        self.objects.iter().find(|entry| entry.object == object)
    }

    /// Total completeness: the weakest object verdict combined with document
    /// reasons. `Complete` reasons still seed the fold so an all-`Complete`
    /// model stays `Complete`.
    pub fn completeness(&self) -> Completeness {
        self.objects.iter().fold(
            DiagnosticReason::combine_all(&self.document),
            |acc, entry| acc.combine(entry.completeness()),
        )
    }

    pub fn summary(&self) -> DiagnosticsSummary {
        let mut complete_objects = 0usize;
        let mut partial_objects = 0usize;
        let mut missing_objects = 0usize;
        let mut unverified_objects = 0usize;
        for entry in &self.objects {
            match entry.completeness() {
                Completeness::Complete => complete_objects += 1,
                Completeness::Partial(_) => partial_objects += 1,
                Completeness::Missing(_) => missing_objects += 1,
                Completeness::Unverified => unverified_objects += 1,
            }
        }
        let reasons =
            self.document.len() + self.objects.iter().map(|o| o.reasons.len()).sum::<usize>();
        let completeness = self.completeness();
        DiagnosticsSummary {
            complete: matches!(completeness, Completeness::Complete),
            completeness,
            objects: self.objects.len(),
            complete_objects,
            partial_objects,
            missing_objects,
            unverified_objects,
            reasons,
        }
    }

    /// Encode the whole model as redacted JSON with every reason preserved.
    pub fn to_json(&self, build_version: &str) -> serde_json::Value {
        let summary = self.summary();
        serde_json::json!({
            "build_version": build_version,
            "completeness": completeness_name(&summary.completeness),
            "summary": {
                "complete": summary.complete,
                "objects": summary.objects,
                "complete_objects": summary.complete_objects,
                "partial_objects": summary.partial_objects,
                "missing_objects": summary.missing_objects,
                "unverified_objects": summary.unverified_objects,
                "reasons": summary.reasons,
            },
            "objects": self.objects.iter().map(ObjectDiagnostics::to_json).collect::<Vec<_>>(),
            "document": self.document.iter().map(DiagnosticReason::to_json).collect::<Vec<_>>(),
            "redacted": true,
        })
    }
}

/// Stable name for a completeness verdict (schema key, not localized).
pub(crate) fn completeness_name(completeness: &Completeness) -> &'static str {
    match completeness {
        Completeness::Complete => "complete",
        Completeness::Partial(_) => "partial",
        Completeness::Missing(_) => "missing",
        Completeness::Unverified => "unverified",
    }
}
