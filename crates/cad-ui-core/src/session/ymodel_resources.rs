//! The `cad_ymodel_resource_preview` / `cad_set_embed_resources` session surface
//! (ADR-0040 D3/D17).
//!
//! Both commands are about the **document's** self-containment, not a user
//! preference and not a drawing command:
//!
//! * `cad_ymodel_resource_preview` answers, read-only, what a save would pack:
//!   the document's current flag plus every external resource it references, with
//!   the size and whether the host could resolve it. It mutates nothing, so a
//!   cancelled save dialog leaves no trace.
//! * `cad_set_embed_resources { embed }` stores the choice on the document, which
//!   is one document revision (`document.json`), exactly like any other edit.
//!
//! A carrier with no `.ymodel` archive refuses both by name
//! (`unsupported-runtime: ymodel`): the flag could never be written and the
//! preview would list resources no file can carry. The Web/`dom-canvas` adapter
//! refuses the setter with `unsupported-runtime: embed_resources` because that is
//! the command it was asked to run.
//!
//! The wire shape is what the save dialog already parses (`save-embed.ts`):
//!
//! ```json
//! { "policy": false,
//!   "entries": [ { "kind": "font", "name": "arial.ttf",
//!                  "source": "/home/u/.local/share/yacr/fonts/arial.ttf",
//!                  "bytes": 1048576, "found": true } ] }
//! ```
//!
//! `bytes` is `0` and `found` is `false` when the resource did not resolve; a row
//! is never dropped because it is missing (the dialog groups exactly those).

use cad_host::save::resources::{ResourceCandidate, ResourcePreview};

use super::{cad_error_text, CadSession};

/// The preview command name (also the browser adapter's refusal subject).
pub const YMODEL_PREVIEW_COMMAND: &str = "cad_ymodel_resource_preview";

/// The setter command name.
pub const EMBED_RESOURCES_COMMAND: &str = "cad_set_embed_resources";

/// The refusal of a carrier whose document is not a `.ymodel` archive.
pub const UNSUPPORTED_YMODEL: &str = "unsupported-runtime: ymodel";

/// The refusal of the setter on a carrier that has no archive to write.
pub const UNSUPPORTED_EMBED_RESOURCES: &str = "unsupported-runtime: embed_resources";

/// One external resource a save would embed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CadYmodelResource {
    /// `font`, `xref` or `image` — the persisted `BundledKind` spelling.
    pub kind: String,
    /// The logical reference the document uses, verbatim.
    pub name: String,
    /// Where it resolved (or was looked for); diagnostics only.
    pub source: Option<String>,
    /// Size in bytes; `0` when it did not resolve.
    pub bytes: u64,
    /// Whether the host can read the bytes right now.
    pub found: bool,
}

impl From<ResourceCandidate> for CadYmodelResource {
    fn from(candidate: ResourceCandidate) -> Self {
        let found = candidate.found();
        Self {
            kind: candidate.kind.as_str().to_string(),
            name: candidate.name,
            source: candidate.source,
            bytes: candidate.size,
            found,
        }
    }
}

/// The `cad_ymodel_resource_preview` answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct CadYmodelResources {
    /// The document's current self-containment flag.
    pub policy: bool,
    pub entries: Vec<CadYmodelResource>,
}

impl From<ResourcePreview> for CadYmodelResources {
    fn from(preview: ResourcePreview) -> Self {
        Self {
            policy: preview.policy,
            entries: preview.entries.into_iter().map(Into::into).collect(),
        }
    }
}

impl CadSession {
    /// What a save would embed, without touching the document (ADR-0040 D17).
    ///
    /// Read-only: no revision, no dirty scope, no file write.
    pub fn ymodel_resource_preview(&self) -> Result<CadYmodelResources, String> {
        self.core
            .ymodel_resource_preview()
            .map(Into::into)
            .map_err(|error| cad_error_text(&error))
    }

    /// Store the save dialog's self-containment choice on the document (D3).
    ///
    /// One document revision and a dirty `document.json`; an unchanged choice is a
    /// documented no-op. A refusal is surfaced through `cad_status.error` (the
    /// dialog must not save a file that ignores the answer), never swallowed.
    pub fn set_embed_resources(&self, embed: bool) -> Result<bool, String> {
        match self.core.set_embed_resources(embed) {
            Ok(changed) => {
                *self.error.borrow_mut() = None;
                Ok(changed)
            }
            Err(error) => {
                let text = cad_error_text(&error);
                *self.error.borrow_mut() = Some(text.clone());
                Err(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact payload the save dialog parses (`apps/desktop-ui/src/commands/
    /// save-embed.ts`): field names, `null` for an unreported source, and a row
    /// for a missing resource instead of a dropped one.
    #[test]
    fn the_preview_wire_shape_is_the_one_the_dialog_parses() {
        let answer = CadYmodelResources {
            policy: false,
            entries: vec![
                CadYmodelResource {
                    kind: "font".to_string(),
                    name: "arial.ttf".to_string(),
                    source: Some("/fonts/arial.ttf".to_string()),
                    bytes: 1024,
                    found: true,
                },
                CadYmodelResource {
                    kind: "image".to_string(),
                    name: "logo.png".to_string(),
                    source: None,
                    bytes: 0,
                    found: false,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&answer).expect("json"),
            serde_json::json!({
                "policy": false,
                "entries": [
                    {
                        "kind": "font",
                        "name": "arial.ttf",
                        "source": "/fonts/arial.ttf",
                        "bytes": 1024,
                        "found": true,
                    },
                    {
                        "kind": "image",
                        "name": "logo.png",
                        "source": null,
                        "bytes": 0,
                        "found": false,
                    },
                ],
            })
        );
    }

    #[test]
    fn the_refusal_markers_are_stable() {
        assert_eq!(UNSUPPORTED_YMODEL, "unsupported-runtime: ymodel");
        assert_eq!(
            UNSUPPORTED_EMBED_RESOURCES,
            "unsupported-runtime: embed_resources"
        );
        assert_eq!(YMODEL_PREVIEW_COMMAND, "cad_ymodel_resource_preview");
        assert_eq!(EMBED_RESOURCES_COMMAND, "cad_set_embed_resources");
    }
}
