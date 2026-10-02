//! Versioned sidecar format and annotation business rules, never DWG writing.
//!
//! Spec v2.0 §3.4, §16.3: annotations live in their own versioned JSON bound to
//! a content fingerprint, are only mutated through the annotation database's
//! transaction path, and preserve unknown fields so a newer writer's data
//! survives an older reader.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use cad_db::{
    AnchorStatus, Annotation, AnnotationDatabase, AnnotationGeometry, AnnotationStyle, ChangeSet,
    EntityAnchor, MeasurementAlgorithm, MeasurementRecord,
};
use cad_domain::*;
use serde_json::{json, Map, Value};

mod annotation;
mod bookmark;
mod geometry;
mod identity;
mod json;
mod mapping;
mod service;
mod space;
mod transform;
mod types;
mod units;
mod uuid;

pub use types::*;

pub(crate) use annotation::*;
pub(crate) use bookmark::*;
pub(crate) use geometry::*;
pub(crate) use identity::*;
pub(crate) use json::*;
pub(crate) use mapping::*;
pub(crate) use space::*;
pub(crate) use transform::*;
pub(crate) use units::*;
pub(crate) use uuid::*;

#[cfg(test)]
mod tests;
