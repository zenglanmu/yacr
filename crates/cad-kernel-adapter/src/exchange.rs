//! The raw solid/surface payload crossing the kernel seam, plus its
//! byte-independent classification.

use super::*;

/// The raw solid/surface payload crossing the kernel seam.
///
/// The payload is kept opaque and byte-oriented on purpose: this adapter is the
/// only place that may ever interpret it, and until an ACIS kernel is linked it
/// stays an unresolved byte buffer.
#[derive(Debug, Clone, PartialEq)]
pub enum SolidExchange {
    /// ACIS SAT text.
    Sat(Vec<u8>),
    /// ACIS SAB binary.
    Sab(Vec<u8>),
    /// Neutral B-rep lifted from a SAT/SAB payload by the importer, the only
    /// acadrust consumer. This is what [`BrepTessellator`] evaluates; the raw
    /// bytes are never handed to a kernel through this variant.
    Brep(BrepData),
    /// Any other source-declared payload (for example an importer-internal
    /// opaque body). It is carried verbatim so a future decoder can be added
    /// without changing the request shape.
    Unsupported { type_key: String, data: Vec<u8> },
}

/// Classification of a [`SolidExchange`], independent of its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeKind {
    Sat,
    Sab,
    Brep,
    Unsupported,
}

impl ExchangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExchangeKind::Sat => "sat",
            ExchangeKind::Sab => "sab",
            ExchangeKind::Brep => "brep",
            ExchangeKind::Unsupported => "unsupported",
        }
    }
}

impl SolidExchange {
    pub fn kind(&self) -> ExchangeKind {
        match self {
            SolidExchange::Sat(_) => ExchangeKind::Sat,
            SolidExchange::Sab(_) => ExchangeKind::Sab,
            SolidExchange::Brep(_) => ExchangeKind::Brep,
            SolidExchange::Unsupported { .. } => ExchangeKind::Unsupported,
        }
    }

    /// Whether the payload carries any bytes at all.
    ///
    /// An empty payload is *not* a valid solid; callers must report it rather
    /// than treat the absence of geometry as success (`kernel.empty_geometry`).
    pub fn is_empty(&self) -> bool {
        match self {
            SolidExchange::Sat(bytes) | SolidExchange::Sab(bytes) => bytes.is_empty(),
            SolidExchange::Brep(brep) => brep.is_empty(),
            SolidExchange::Unsupported { data, .. } => data.is_empty(),
        }
    }

    /// Byte length of the payload, for budget and diagnostic reporting.
    ///
    /// A neutral B-rep has no byte payload of its own; its size is its face
    /// count, so the byte-oriented callers still get a meaningful number.
    pub fn len(&self) -> usize {
        match self {
            SolidExchange::Sat(bytes) | SolidExchange::Sab(bytes) => bytes.len(),
            SolidExchange::Brep(brep) => brep.face_count(),
            SolidExchange::Unsupported { data, .. } => data.len(),
        }
    }

    /// A sanitised logical key for diagnostics; never a raw payload.
    pub fn type_key(&self) -> &str {
        match self {
            SolidExchange::Sat(_) => "acis.sat",
            SolidExchange::Sab(_) => "acis.sab",
            SolidExchange::Brep(_) => "acis.brep",
            SolidExchange::Unsupported { type_key, .. } => type_key,
        }
    }
}
