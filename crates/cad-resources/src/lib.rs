//! Logical keys only; explicit grants precede any external access.
//!
//! Spec v2.0 §7.3, §16.2: references are logical keys, never platform paths. A
//! crafted drawing must not be able to direct the app outside its allowed
//! roots, so every reference is sanitised to a bare file name and the resolver
//! chain is explicit and ordered.

use cad_domain::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

mod bundle;
mod font;
mod resolver;

pub use bundle::*;
pub use font::*;
pub use resolver::*;

#[cfg(test)]
mod tests;
