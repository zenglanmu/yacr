//! Host-owned, data-only viewer configuration and pure shell presentation.
//!
//! The protocol is defined by `docs/ui-spec/ui-desc.md` and must never contain
//! executable script or arbitrary callbacks: every field is plain data. Parsing
//! order is "built-in defaults → preset → host override → host-allowed user
//! preference"; missing fields inherit, explicit `false` survives, arrays are
//! replaced wholesale and user preferences may never re-enable a capability or
//! component the host/preset disabled (they clamp, they do not error).
//!
//! The implementation is split across the submodules of this directory; every
//! moved item is re-exported here under its original name, so the module path
//! `crate::viewer_config::…` (and therefore the crate's public surface) is
//! unchanged.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod colour;
mod commands;
mod error;
mod merge;
mod presentation;
mod schema;
mod store;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_display;

pub use colour::*;
pub use commands::*;
pub use error::*;
pub use presentation::*;
pub use schema::*;
pub use store::*;

use commands::{preset_command_visible, preset_components};
use merge::{allowed_path_is_known, apply_preference, deep_merge_create, merge, project_allowed};
