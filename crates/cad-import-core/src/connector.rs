//! Format connector descriptors: the format-neutral catalogue vocabulary.
//!
//! A *connector* is one drawing format seen from both directions: what it can
//! import, what it can export, and how faithful that export is. The concrete
//! dispatch table (which reader/writer runs for which bytes) lives in
//! `cad-app`'s registry; this module only defines the shared descriptor the
//! registry, the reports and (later) the UI name.
//!
//! Nothing here knows a parser crate or a UI toolkit, so a descriptor can be a
//! `const` in the format adapter that owns it.

/// How faithful an output direction is to the `cad-db` document it projects.
///
/// Only two levels exist on purpose: there is no "L3" yet, and a format that
/// cannot be projected exactly must say [`FidelityLevel::L1`] rather than claim
/// a lossless round trip. `L1` is a **Save As**: the exporter reports every
/// conversion and drop (see [`crate::export::CompatReport`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidelityLevel {
    /// Byte/semantic pass-through of the source representation (not implemented
    /// for any format in this build; declared so the level exists by name).
    L0,
    /// Resolved-display projection with an explicit per-object report.
    L1,
}

/// Which direction one compatibility record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportDirection {
    Import,
    Export,
}

/// What one connector can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportCapability {
    /// Version/range string as the reader actually accepts it (for example
    /// `"AC1014..AC1032"`). It is documentation, never parsed by code.
    pub versions: &'static str,
    /// Container spelling (`"dwg"`, `"dxf"`, `"step"`, `"ole"`, `"zip"`, ...).
    pub container: &'static str,
}

/// What one connector can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportCapability {
    /// Version/range string the writer emits.
    pub versions: &'static str,
    /// Fidelity of the written projection.
    pub level: FidelityLevel,
    /// `true` when the writer cannot represent every document faithfully, so a
    /// caller must read the per-object report instead of assuming a round trip.
    pub lossy: bool,
}

/// One drawing format as a connector: identity, extensions and both directions.
///
/// Descriptors are `const`s owned by the adapter crate that implements the
/// format, so there is exactly one declaration per format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorDescriptor {
    /// Stable machine id (`"dxf"`, `"dwg"`, `"step"`, ...). Also the prefix of
    /// the format's refusal codes.
    pub id: &'static str,
    /// i18n key for the user-facing name (the descriptor never carries prose).
    pub name_key: &'static str,
    /// Lower-case file extensions without the dot, as the picker shows them.
    pub extensions: &'static [&'static str],
    /// `None` when the format has no reader.
    pub import: Option<ImportCapability>,
    /// `None` when the format has no writer (an export attempt is then a named
    /// refusal, never a silent success).
    pub export: Option<ExportCapability>,
    /// Type keys the connector declares it models, for capability reporting.
    pub declared_classes: &'static [&'static str],
}

/// A type that carries one connector descriptor.
///
/// Implemented by the format adapters (`AcadrustExporter`, ...) so a caller can
/// ask an object what format it is without knowing the concrete type.
pub trait Connector {
    /// The descriptor this object implements.
    fn descriptor(&self) -> &'static ConnectorDescriptor;
}
