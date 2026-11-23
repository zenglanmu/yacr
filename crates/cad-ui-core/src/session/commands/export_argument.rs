//! `EXPORT <TOKEN>`: the typed argument form of the one `EXPORT` selector
//! (ADR-0052).
//!
//! Bare `EXPORT` keeps the host-owned all-format picker: the user chooses the
//! destination and the **chosen extension** decides the writer, exactly as
//! before. A typed token fixes which writer this one interaction may use, and
//! only changes the picker's suggested extension (`<stem>.<token extension>`):
//!
//! | token | registry writer | suggested extension |
//! | --- | --- | --- |
//! | `DXF` | `dxf` | `.dxf` |
//! | `DWG` | `dwg` (experimental) | `.dwg` |
//! | `STL` | `stl` | `.stl` |
//! | `OBJ` | `obj` | `.obj` |
//! | `PLY` | `ply` | `.ply` |
//! | `OFF` | `off` | `.off` |
//! | `COFF` | `off` (colour-capable header) | `.coff` |
//! | `IFC` | `ifc` | `.ifc` |
//! | `SKP` | `skp` (experimental) | `.skp` |
//! | `GLB` | `glb` (binary glTF) | `.glb` |
//! | `GLTF` | `gltf` (JSON glTF) | `.gltf` |
//!
//! The token is matched ASCII case-insensitively after trimming. The registry
//! table and the token grammar live in [`cad_host::policy`] so the host and the
//! session cannot drift; this module only resolves the word and hands the writer
//! pair to [`cad_host::HostCore::request_export_format`].
//!
//! Stable refusal: `export.unknown_format: …` for a missing, multi-word or
//! unknown token. It happens **before** the host is touched, so no picker opens
//! and nothing is written. A target that disagrees with the forced writer is a
//! separate host refusal (`export.format_mismatch: …`) raised before the write.
//!
//! This module lives in its own file only to keep `commands.rs` and
//! `session/mod.rs` within the code-size gate.

use cad_host::policy::{self, ExportFormatSpec};

use super::super::diagnostics::cad_error_text;
use super::super::CadSession;
use super::EXPORT_SELECTOR;

/// The stable marker prefix of an `EXPORT <TOKEN>` refusal.
pub const EXPORT_FORMAT_REASON_PREFIX: &str = "export.unknown_format";

/// Parse `EXPORT <TOKEN>`: exactly one word naming a writable export format.
///
/// The whole remainder is the argument (the splitter already trimmed it), so a
/// second word (`EXPORT GLB extra`) is malformed rather than silently ignored.
pub fn parse_export_argument(argument: &str) -> Option<&'static ExportFormatSpec> {
    let mut words = argument.split_whitespace();
    let token = words.next()?;
    if words.next().is_some() {
        return None;
    }
    policy::export_format_for_token(token)
}

/// The stable refusal for an argument outside the `EXPORT <TOKEN>` grammar.
pub fn export_format_reason(text: &str) -> String {
    format!(
        "{EXPORT_FORMAT_REASON_PREFIX}: `{}` is not one writable EXPORT format; the typed form is \
         `EXPORT <TOKEN>` with TOKEN one of DXF DWG STL OBJ PLY OFF COFF IFC SKP GLB GLTF (a bare \
         EXPORT opens the full picker)",
        text.trim()
    )
}

impl CadSession {
    /// Apply one typed `EXPORT <TOKEN>` line.
    ///
    /// The host still opens the picker (the destination is the user's choice),
    /// but the writer is fixed for this interaction; the chosen extension remains
    /// the final authority and a mismatch is refused by the host before any
    /// write. A successful request is recorded in the command log exactly like
    /// `TABLEEXPORT <path>`'s host write.
    pub(in crate::session) fn export_argument(
        &self,
        argument: &str,
        text: &str,
    ) -> Result<(), String> {
        let Some(spec) = parse_export_argument(argument) else {
            return self.refuse_argument(export_format_reason(text));
        };
        if let Err(error) = self
            .core
            .request_export_format(spec.format_id, spec.extension)
        {
            // The host graded its own status (`save.export_failed` with the real
            // busy/unsupported reason); the session only mirrors the text.
            return self.refuse_argument(cad_error_text(&error));
        }
        self.core.record_command(EXPORT_SELECTOR);
        *self.error.borrow_mut() = None;
        Ok(())
    }
}
