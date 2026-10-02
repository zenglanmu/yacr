//! Headless core CLI. Every data operation shares the application/database
//! command path used by the UI; there is no second business implementation.
//!
//! Spec v2.0 §19: operations return structured JSON with object IDs, transaction
//! results and diagnostics — never an opaque success string. Diagnostics are
//! redacted by `cad-diagnostics`; fixed-viewport rendering explicitly requires
//! a GPU environment and otherwise reports "not run".
//!
//! Machine contract (audit B30, N01 §5.3):
//!
//! * stdout carries **only** the pretty-printed JSON result document. Every
//!   human-facing string lives on stderr, so `--locale` can never change a
//!   machine key or value.
//! * On any failure the process prints
//!   `{schema_version, operation, error:{code,message,context}}` to stderr and
//!   exits non-zero. `code` is a stable machine key; `message` is localized.
//! * `--out <file>` writes the JSON result atomically (same-directory temp file
//!   followed by a rename), so a failed run never leaves a partial file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cad_app::host::HostController;
use cad_app::{Command, CommandId, CommandPayload};
use cad_domain::*;
mod invocation;
mod io;
mod ops;
mod report;
mod run;

pub use invocation::*;
pub(crate) use io::*;
pub(crate) use ops::*;
pub(crate) use report::*;
pub use run::*;

#[cfg(test)]
mod tests;
