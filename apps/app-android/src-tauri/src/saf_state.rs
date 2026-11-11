//! The SAF result → host event mapping, kept in its own file so
//! `platform.rs` stays under the repository's 1000-line code-file ceiling.
//!
//! The Android-only `crate::saf` bridge produces the raw result; this module owns
//! the purpose→event mapping (and the cold-start rule) so both are unit-testable
//! on the development host without a device.

use std::sync::Arc;

use cad_platform::{FileEvent, FilePurpose, FileTarget};

/// The terminal/pending outcome of one SAF interaction.
///
/// Defined here (behind the same `cfg` as [`saf_event`]) so the purpose→event
/// mapping is unit-testable on the development host; the Android-only
/// `crate::saf` module produces it.
#[cfg(any(target_os = "android", test))]
#[derive(Debug)]
pub(crate) enum SafState {
    /// The picker/read has not completed (state `NONE`/`LOADING`).
    Pending,
    /// The user dismissed the picker (distinct from a failure).
    Cancelled,
    /// The interaction failed; the string is the raw (untranslated) reason.
    Failed(String),
    /// An open target was chosen and its bytes read.
    Opened {
        uri: String,
        name: String,
        bytes: Vec<u8>,
    },
    /// A save target was chosen (no bytes; writing happens later).
    SaveTarget { uri: String, name: String },
}

/// Map an armed purpose and a SAF result to the event to queue.
///
/// `None` means "still pending, queue nothing". A result whose kind does not
/// match the armed purpose is an explicit [`FileEvent::Failed`] — never a
/// fabricated open/save (an open-chosen URI must never be written as a DXF).
#[cfg(any(target_os = "android", test))]
pub(crate) fn saf_event(purpose: FilePurpose, result: SafState) -> Option<FileEvent> {
    match (purpose, result) {
        (_, SafState::Pending) => None,
        (purpose, SafState::Cancelled) => Some(FileEvent::Cancelled { purpose }),
        (purpose, SafState::Failed(reason)) => Some(FileEvent::Failed { purpose, reason }),
        (FilePurpose::Open, SafState::Opened { uri, name, bytes }) => Some(FileEvent::Opened {
            target: FileTarget {
                display_name: name,
                opaque: uri,
            },
            bytes: Arc::from(bytes.into_boxed_slice()),
        }),
        (FilePurpose::Save(save), SafState::SaveTarget { uri, name }) => {
            Some(FileEvent::SaveTarget {
                purpose: save,
                target: FileTarget {
                    display_name: name,
                    opaque: uri,
                },
            })
        }
        (FilePurpose::Open, SafState::SaveTarget { .. }) => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason: "the document picker returned a save target for an open request".into(),
        }),
        (FilePurpose::Save(_), SafState::Opened { .. }) => Some(FileEvent::Failed {
            purpose,
            reason: "the document picker returned an open target for a save request".into(),
        }),
    }
}

/// Map a result with **nothing armed** (an external/cold-start delivery).
///
/// Only an opened document is accepted without an armed request; a save target
/// in that position is an explicit `Failed { Open }` (there is no save request it
/// could belong to), and a cancel is ignored.
#[cfg(any(target_os = "android", test))]
pub(crate) fn cold_event(result: SafState) -> Option<FileEvent> {
    match result {
        SafState::Pending | SafState::Cancelled => None,
        SafState::Opened { uri, name, bytes } => Some(FileEvent::Opened {
            target: FileTarget {
                display_name: name,
                opaque: uri,
            },
            bytes: Arc::from(bytes.into_boxed_slice()),
        }),
        SafState::SaveTarget { .. } => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason: "a save target was delivered without an armed save request".into(),
        }),
        SafState::Failed(reason) => Some(FileEvent::Failed {
            purpose: FilePurpose::Open,
            reason,
        }),
    }
}
