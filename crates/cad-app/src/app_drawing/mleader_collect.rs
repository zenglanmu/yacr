//! The adoption bookkeeping for `MLEADERCOLLECT` (see
//! [`super::mleader_edit::Application::mleader_collect`]).
//!
//! COLLECT accepts a selection whose leaders disagree on style, `90` override
//! mask or plane frame. Everything the DXF format can carry on the merged lines
//! themselves — per-line path type (`LEADER_LINE 170`, override word `93` bit 1)
//! and arrowhead size (`40`, bit 16) — is written as a per-line override. The
//! rest has **no per-line group** (annotation text height `CONTEXT_DATA 41`,
//! landing gap `145`, text style `343`, entity scale `45`, the landing/dogleg
//! enable flags `290`/`291`, and the one plane frame `110`/`111`/`112`/`297`), so
//! it is adopted from the base leader and every adopted value is named in the
//! command's report. This module owns that bookkeeping so the command file stays
//! inside the code-size limit.

use super::mleader_edit::MLEADER_EDIT_EPS;
use super::*;
use cad_domain::{
    MLeaderLine, MLeaderPathType, MLeaderStyleValues, MLeaderText, MultiLeaderGeometry, PlaneFrame,
};

/// Two measures are equal within the shared relative tolerance.
pub(super) fn same_measure(left: f64, right: f64) -> bool {
    (left - right).abs() <= MLEADER_EDIT_EPS * (1.0 + left.abs().max(right.abs()))
}

/// The path type one leader line actually draws with (its own when the
/// override word carries [`MLeaderLine::PATH_TYPE`], the entity's otherwise).
pub(super) fn effective_line_path(
    leader: &MultiLeaderGeometry,
    line: &MLeaderLine,
) -> MLeaderPathType {
    if line.override_flags & MLeaderLine::PATH_TYPE != 0 {
        line.path_type
    } else {
        leader.style().path_type
    }
}

/// The arrowhead size one leader line actually draws with (its own when the
/// override word carries [`MLeaderLine::ARROWHEAD_SIZE`], the entity's
/// otherwise).
pub(super) fn effective_line_arrowhead(leader: &MultiLeaderGeometry, line: &MLeaderLine) -> f64 {
    if line.override_flags & MLeaderLine::ARROWHEAD_SIZE != 0 {
        line.arrowhead_size
    } else {
        leader.style().arrowhead_size
    }
}

/// What [`Application::mleader_collect`] adopts from the base leader, and the
/// evidence that nothing was adopted silently.
///
/// The bookkeeping exists so the command can accept a heterogeneous selection
/// (AutoCAD's COLLECT is a user request to merge) while still telling the user
/// exactly which values changed hands. Every field recorded here has **no
/// per-line DXF group**, which is why it cannot travel on the merged lines; the
/// per-line values that *do* have a group (path type `170`, arrowhead size `40`)
/// are written as overrides instead and are only mentioned when they were.
pub(super) struct CollectAdoption {
    base_style: MLeaderStyleValues,
    base_text: MLeaderText,
    base_style_name: String,
    base_mask: u32,
    /// The base snapshot's effective path type (the merged entity's own value).
    pub(super) path: MLeaderPathType,
    /// The base snapshot's effective arrowhead size.
    pub(super) arrowhead: f64,
    /// The base leader's plane frame (the merged entity's frame).
    base_plane: PlaneFrame,
    /// Entities whose style name, mask or snapshot differed.
    style_sources: Vec<u128>,
    /// The adopted snapshot fields, deduplicated and in the order found.
    fields: Vec<&'static str>,
    /// Entities whose plane frame differed.
    plane_sources: Vec<u128>,
}

impl CollectAdoption {
    pub(super) fn new(base: &MultiLeaderGeometry) -> Self {
        CollectAdoption {
            base_style: base.style().clone(),
            base_text: base.text().clone(),
            base_style_name: base.style_name().to_string(),
            base_mask: base.override_mask(),
            path: base.style().path_type,
            arrowhead: base.style().arrowhead_size,
            base_plane: base.plane(),
            style_sources: Vec::new(),
            fields: Vec::new(),
            plane_sources: Vec::new(),
        }
    }

    /// Record one field that the collected leader has to adopt from its base.
    pub(super) fn note_field(&mut self, field: &'static str, entity: u128) {
        if !self.style_sources.contains(&entity) {
            self.style_sources.push(entity);
        }
        if !self.fields.contains(&field) {
            self.fields.push(field);
        }
    }

    /// Compare one source leader's snapshot with the base's.
    ///
    /// Path type, arrowhead size and the per-root dogleg length are **not**
    /// recorded: the first two travel as per-line overrides and the third is a
    /// per-root field (`LEADER` `40`) that the cloned root carries. Only the
    /// per-entity fields with no smaller home are adopted.
    pub(super) fn note_style(&mut self, leader: &MultiLeaderGeometry, entity: u128) {
        let style = leader.style();
        let text = leader.text();
        let base_style = &self.base_style;
        let base_text = &self.base_text;
        let mut fields: Vec<&'static str> = Vec::new();
        if !same_measure(text.height, base_text.height) {
            fields.push("annotation text height (CONTEXT_DATA 41)");
        }
        if text.left_attachment != base_text.left_attachment
            || text.attachment_point != base_text.attachment_point
        {
            fields.push("text attachment (CONTEXT_DATA 1 / 174)");
        }
        if !same_measure(style.landing_gap, base_style.landing_gap) {
            fields.push("landing gap (CONTEXT_DATA 145)");
        }
        if style.text_style != base_style.text_style
            || style.text_style_name != base_style.text_style_name
            || style.text_font != base_style.text_font
        {
            fields.push("text style (CONTEXT_DATA 343)");
        }
        if !same_measure(style.scale_factor, base_style.scale_factor) {
            fields.push("entity scale (entity-tail 45)");
        }
        if style.enable_landing != base_style.enable_landing
            || style.enable_dogleg != base_style.enable_dogleg
        {
            fields.push("landing/dogleg enable flags (entity-tail 290 / 291)");
        }
        // The root's own dogleg length travels with the root; the style value is
        // only the fallback for a root that carries none.
        if !same_measure(style.landing_distance, base_style.landing_distance)
            && leader
                .roots()
                .iter()
                .any(|root| !root.dogleg_length.is_finite() || root.dogleg_length <= 1e-9)
        {
            fields.push("default dogleg length (style 43 fallback)");
        }
        let style_name_differs = leader.style_name() != self.base_style_name;
        let mask_differs = leader.override_mask() != self.base_mask;
        if fields.is_empty() && !style_name_differs && !mask_differs {
            return;
        }
        if mask_differs {
            self.note_field("the 90 property-override mask", entity);
        }
        for field in fields {
            self.note_field(field, entity);
        }
        if style_name_differs && !self.style_sources.contains(&entity) {
            self.style_sources.push(entity);
        }
    }

    /// Record a source leader whose plane frame differs from the base's.
    pub(super) fn note_plane(&mut self, leader: &MultiLeaderGeometry, entity: u128) {
        if leader.plane() != self.base_plane {
            self.plane_sources.push(entity);
        }
    }

    /// The adoption report: empty when every source matched the base.
    pub(super) fn report(&self, carried_per_line: bool) -> Vec<Diagnostic> {
        let mut rows = Vec::new();
        if !self.style_sources.is_empty() {
            let entities = ids(&self.style_sources);
            let fields = if self.fields.is_empty() {
                "no per-entity snapshot field differed".to_string()
            } else {
                self.fields.join(", ")
            };
            let carried = if carried_per_line {
                " Per-line path type (leader-line 170 with override word 93 bit 1) and arrowhead \
                 size (40 with bit 16) differences are written as per-line overrides, so those \
                 lines keep the appearance they had in their own leader."
            } else {
                ""
            };
            rows.push(Diagnostic {
                object: None,
                code: "mleadercollect.style_adopted".into(),
                message: format!(
                    "MLEADERCOLLECT: {entities} did not match the base leader's MLEADERSTYLE \
                     '{name}' / 90 override mask {mask:#X} and were collected rather than \
                     refused. The collected leader keeps the base snapshot, and these \
                     per-entity fields now draw with the base value (each has no per-line DXF \
                     group, so it cannot travel on the merged lines): {fields}.{carried}",
                    name = self.base_style_name,
                    mask = self.base_mask,
                ),
            });
        }
        if !self.plane_sources.is_empty() {
            rows.push(Diagnostic {
                object: None,
                code: "mleadercollect.plane_adopted".into(),
                message: format!(
                    "MLEADERCOLLECT: {} lie on a different plane frame (110/111/112/297); one \
                     collected leader has one frame, so the base leader's is kept. Every root \
                     landing, leader-line vertex and the annotation position are carried in \
                     world coordinates unchanged — these entities' frame metadata is the base \
                     one's",
                    ids(&self.plane_sources)
                ),
            });
        }
        rows
    }
}

/// A readable `entity 3, entity 7` list.
fn ids(sources: &[u128]) -> String {
    sources
        .iter()
        .map(|id| format!("entity {id}"))
        .collect::<Vec<_>>()
        .join(", ")
}
