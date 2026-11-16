//! The per-line override edits of `MLEADEREDIT` (see
//! [`super::mleader_edit::Application::mleader_edit`]).
//!
//! A leader line owns exactly two DXF fields behind its `93` override word that
//! this model stores on the line itself: its **path type** (`LEADER_LINE 170`,
//! bit `1`) and its **arrowhead size** (`40`, bit `16`). These helpers set one of
//! them and stamp its bit, or clear the bit and re-stamp the value from the
//! resolved style — always by rebuilding the definition through
//! [`MultiLeaderGeometry::new`], so the stored display can never disagree with it.
//!
//! Split out of `mleader_edit.rs` for the code-size gate. Every helper returns
//! `Ok(None)` for the declared no-op (the stored line already carries the
//! requested state) and a named `Err` for a refusal; the caller decides whether
//! the whole selection is staged.

use super::mleader_collect::same_measure;
use super::mleader_edit::{invalid, parts_of};
use super::*;
use crate::command_payload::MLeaderLineOverrideField;
use cad_domain::{MLeaderLine, MLeaderPathType, MultiLeaderGeometry};

/// The named out-of-range refusal for one leader line index.
pub(super) fn line_index_refusal(leader: &MultiLeaderGeometry, line_index: usize) -> CadError {
    invalid(
        "mleaderedit",
        "index",
        format!(
            "leader line index {line_index} is out of range; the leader stores {} line(s)",
            leader.lines().len()
        ),
    )
}

/// Set one line's own path type (`LEADER_LINE 170`) and stamp override word bit
/// `1`.
///
/// The per-line override is what makes the value survive a reload: the writer
/// emits `path_type` verbatim and the `93` bit is what tells the reader (and the
/// display) to prefer it over the entity/style value.
pub(super) fn set_line_path_type(
    leader: &MultiLeaderGeometry,
    line_index: usize,
    path_type: MLeaderPathType,
) -> CadResult<Option<MultiLeaderGeometry>> {
    let line = leader
        .lines()
        .get(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    if line.override_flags & MLeaderLine::PATH_TYPE != 0 && line.path_type == path_type {
        return Ok(None);
    }
    let mut parts = parts_of(leader);
    let line = parts
        .lines
        .get_mut(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    line.path_type = path_type;
    line.override_flags |= MLeaderLine::PATH_TYPE;
    MultiLeaderGeometry::new(parts).map(Some)
}

/// Set one line's own arrowhead size (`LEADER_LINE 40`) and stamp override word
/// bit `16`.
///
/// A non-finite or non-positive size is refused by name: the display draws no
/// arrowhead for a non-positive per-line size while the entity/style value would,
/// and a stored `0` has no unambiguous meaning in the format — inventing either
/// reading would be a lie.
pub(super) fn set_line_arrowhead_size(
    leader: &MultiLeaderGeometry,
    line_index: usize,
    size: f64,
) -> CadResult<Option<MultiLeaderGeometry>> {
    if !size.is_finite() || size <= 0.0 {
        return Err(invalid(
            "mleaderedit",
            "arrowhead_size",
            format!("the per-line arrowhead size must be finite and strictly positive, got {size}"),
        ));
    }
    let line = leader
        .lines()
        .get(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    if line.override_flags & MLeaderLine::ARROWHEAD_SIZE != 0
        && same_measure(line.arrowhead_size, size)
    {
        return Ok(None);
    }
    let mut parts = parts_of(leader);
    let line = parts
        .lines
        .get_mut(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    line.arrowhead_size = size;
    line.override_flags |= MLeaderLine::ARROWHEAD_SIZE;
    MultiLeaderGeometry::new(parts).map(Some)
}

/// Clear one line's override bit and re-stamp the stored value from the resolved
/// style, so the entity/style value applies again.
///
/// Without this the override word would be one-way: a line set to `SPLINE` could
/// never follow its style again. The stored value is re-stamped (not left stale)
/// so the field never holds a value the style could not reach.
pub(super) fn clear_line_override(
    leader: &MultiLeaderGeometry,
    line_index: usize,
    field: MLeaderLineOverrideField,
) -> CadResult<Option<MultiLeaderGeometry>> {
    let line = leader
        .lines()
        .get(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    if line.override_flags & field.bit() == 0 {
        return Ok(None);
    }
    let style = leader.style().clone();
    let mut parts = parts_of(leader);
    let line = parts
        .lines
        .get_mut(line_index)
        .ok_or_else(|| line_index_refusal(leader, line_index))?;
    line.override_flags &= !field.bit();
    match field {
        MLeaderLineOverrideField::PathType => line.path_type = style.path_type,
        MLeaderLineOverrideField::ArrowheadSize => line.arrowhead_size = style.arrowhead_size,
    }
    MultiLeaderGeometry::new(parts).map(Some)
}
