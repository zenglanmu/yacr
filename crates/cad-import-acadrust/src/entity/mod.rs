//! Entity, block and hatch conversion methods for [`ImporterBuilder`].
//!
//! Decomposed into cohesive submodules, one item group each. The module path
//! `crate::entity` and every [`ImporterBuilder`] method name/visibility are
//! unchanged; only the file layout moved.

use super::*;

mod convert;
mod extended;
mod hatch;
mod insert;
mod push;
mod table;

// Re-export every moved module-scope item so the paths that resolved inside the
// former single-file `entity` module keep resolving. Most of these items are
// inherent `ImporterBuilder` methods, which are reachable through the type
// regardless; the allow keeps the uniform surface from tripping `-D warnings`.
#[allow(unused_imports)]
use convert::*;
#[allow(unused_imports)]
use extended::*;
#[allow(unused_imports)]
use hatch::*;
#[allow(unused_imports)]
use insert::*;
#[allow(unused_imports)]
use push::*;
#[allow(unused_imports)]
use table::*;
