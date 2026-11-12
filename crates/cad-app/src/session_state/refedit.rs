//! The bounded, session-only state of one in-place block-reference edit
//! (`REFEDIT`).
//!
//! Split out of `session_state.rs` to keep that file inside the code-size
//! ceiling; the public path is `cad_app::session_state::RefEditSession` (and
//! re-exported through `cad_app::RefEditSession`).
//!
//! # What this is (and is not)
//!
//! A `REFEDIT` session is **derived working state**, never a second drawing and
//! never a second block authority. The source INSERT, its block definition and
//! the extracted model-space clones all live in `cad-db`; this struct only
//! remembers *which* clone belongs to which block member and *which* entities
//! are currently in the working set. `REFCLOSE SAVE` writes the block table
//! through [`cad_db::DrawingDatabase::apply_drawing_changes_with_blocks`], the
//! single block write path; `REFCLOSE DISCARD` deletes the clones.
//!
//! # Boundaries (v1)
//!
//! * Exactly one source INSERT and one block definition per session; a second
//!   `REFEDIT` while active is the named `refedit.active` refusal.
//! * Only **visible** block members are extracted (the same rule `NCOPY` and
//!   `EXPLODE` use). A member hidden by the block's active dynamic-visibility
//!   state has no clone and is left untouched on save.
//! * There is no reference fade/lock in this build, so host objects stay fully
//!   pickable while a session is active; `REFSET` is the explicit way to add
//!   them to the working set.
//! * The session dies with the document (the host rebuilds the session on new
//!   content), and it is cleared by `REFCLOSE SAVE|DISCARD`.

use std::collections::{BTreeMap, BTreeSet};

use cad_db::EntityDisplayState;
use cad_domain::{BlockId, DocumentId, EntityId, SelectionRef, Transform3};

/// One source block member captured at `REFEDIT` time.
///
/// `display` is the member's full display state (render attributes and
/// block/visibility membership) before the edit; it is recorded so an undo of a
/// save can restore a removed member exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct RefEditMember {
    /// The block-space entity id of the member.
    pub original: EntityId,
    /// The display state the member had when the session began.
    pub display: EntityDisplayState,
}

/// The data of one active in-place block edit.
#[derive(Debug, Clone, PartialEq)]
pub struct RefEditActive {
    /// The open document this edit belongs to.
    pub document: DocumentId,
    /// The source model-space INSERT the session was started from.
    pub source: SelectionRef,
    /// The block definition being edited.
    pub block: BlockId,
    /// The source INSERT's placement (a positive-determinant similarity).
    ///
    /// Stored at begin time so every extracted clone can be inverse-transformed
    /// exactly, even if the source INSERT row is erased while the session is
    /// active.
    pub insert_transform: Transform3,
    /// Working clone id → the block member it was extracted from.
    ///
    /// A clone in this map is a *representation* of an existing member; a
    /// working id **not** in this map is an added host object.
    pub clone_of: BTreeMap<EntityId, EntityId>,
    /// The entities currently in the working set (extracted clones plus added
    /// host objects).
    pub working: BTreeSet<EntityId>,
    /// The visible source members and their pre-edit display states.
    pub members: Vec<RefEditMember>,
}

/// The session's in-place block-edit state: at most one active edit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefEditSession {
    active: Option<RefEditActive>,
}

impl RefEditSession {
    /// Whether an in-place block edit is active.
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// The active edit's data, if any.
    pub fn active(&self) -> Option<&RefEditActive> {
        self.active.as_ref()
    }

    /// Mutable access to the active edit's data, if any.
    pub fn active_mut(&mut self) -> Option<&mut RefEditActive> {
        self.active.as_mut()
    }

    /// Start a session (the caller has already validated it).
    pub(crate) fn begin(&mut self, active: RefEditActive) {
        self.active = Some(active);
    }

    /// Clear the session; a no-op when none is active.
    pub(crate) fn clear(&mut self) {
        self.active = None;
    }

    /// The block member a working clone was extracted from, if it is a clone.
    pub fn clone_of(&self, clone: EntityId) -> Option<EntityId> {
        self.active.as_ref()?.clone_of.get(&clone).copied()
    }

    /// The current working set, if a session is active.
    pub fn working(&self) -> Option<&BTreeSet<EntityId>> {
        self.active.as_ref().map(|active| &active.working)
    }
}
