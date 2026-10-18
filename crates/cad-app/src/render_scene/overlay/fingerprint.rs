use super::*;

/// A stable change fingerprint of the transient **overlay batch** inputs.
///
/// The controller uses it to rebuild **only** the sub-element/preview overlay
/// when a face selection or a preview point/cursor changes, leaving the base
/// drawing `Arc` untouched. Entity-level selection/hover/hidden are the renderer's
/// flags path, not overlay batches, so they are deliberately **excluded** here
/// (see [`entity_flags_fingerprint`]): an entity-only selection change must not
/// rebuild or re-upload any overlay geometry.
pub fn overlay_fingerprint(inputs: &OverlayInputs) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Only sub-element (mesh face) refs still produce an overlay batch.
    for reference in inputs
        .selection
        .refs()
        .iter()
        .filter(|reference| reference.sub_element.is_some())
    {
        reference.document.0.hash(&mut hasher);
        reference.entity.0.hash(&mut hasher);
        for instance in &reference.instance.0 {
            instance.0.hash(&mut hasher);
        }
        // A separator keeps `[a]` distinct from `[]` + `a`.
        u8::MAX.hash(&mut hasher);
        match &reference.sub_element {
            Some(sub) => {
                sub.source_key.hash(&mut hasher);
                sub.topology_revision.0.hash(&mut hasher);
            }
            None => 0u8.hash(&mut hasher),
        }
    }
    hash_preview(
        &mut hasher,
        0u8,
        inputs
            .measurement
            .as_ref()
            .map(|p| (p.points.as_slice(), p.cursor)),
    );
    hash_preview(
        &mut hasher,
        1u8,
        inputs
            .draw_preview
            .as_ref()
            .map(|p| (p.points.as_slice(), p.cursor)),
    );
    // Snap hints are pure session state; their point/kind changes the marker
    // set and must invalidate the transient overlay.
    2u8.hash(&mut hasher);
    inputs.snap_hints_input.len().hash(&mut hasher);
    for hint in &inputs.snap_hints_input {
        // `SnapKind` has no `Hash`; its stable string is the fingerprint key.
        hint.kind.as_str().hash(&mut hasher);
        hash_point(&mut hasher, hint.point);
    }
    // Visibility is part of the inputs: turning an overlay off must rebuild the
    // transient overlay even when the selection and previews are unchanged.
    inputs.visibility.axes.hash(&mut hasher);
    inputs.visibility.grid.hash(&mut hasher);
    inputs.visibility.selection_highlight.hash(&mut hasher);
    inputs.visibility.snap_hints.hash(&mut hasher);
    match inputs.viewport_bounds {
        Some((min, max)) => {
            1u8.hash(&mut hasher);
            hash_point(&mut hasher, min);
            hash_point(&mut hasher, max);
        }
        None => 0u8.hash(&mut hasher),
    }
    hasher.finish()
}

/// A stable change fingerprint of the **entity flags** inputs.
///
/// Entity-level selection/hover/hidden are resolved into renderer flags, not
/// overlay batches, so they are hashed here (not in [`overlay_fingerprint`]).
/// The controller uses it to republish the flags without rebuilding or
/// re-uploading any overlay geometry: an entity-only selection change leaves the
/// base `Arc` and the overlay delta byte-identical.
///
/// `selection_highlight` gates the selected tint only; hover/hidden are
/// independent transient states and always participate.
///
/// Note: `selection_highlight` is hashed **here and** in [`overlay_fingerprint`]
/// on purpose — it gates both the sub-element overlay (built only when on) and
/// the entity-level selected tint, so a toggle must rebuild/republish both.
pub fn entity_flags_fingerprint(inputs: &OverlayInputs) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let hash_refs =
        |hasher: &mut std::collections::hash_map::DefaultHasher, tag: u8, refs: &[SelectionRef]| {
            tag.hash(hasher);
            for reference in refs
                .iter()
                .filter(|reference| reference.sub_element.is_none())
            {
                reference.document.0.hash(hasher);
                reference.entity.0.hash(hasher);
                for instance in &reference.instance.0 {
                    instance.0.hash(hasher);
                }
                u8::MAX.hash(hasher);
            }
        };
    hash_refs(&mut hasher, 0, inputs.selection.refs());
    hash_refs(&mut hasher, 1, inputs.hovered.refs());
    hash_refs(&mut hasher, 2, inputs.hidden.refs());
    inputs.visibility.selection_highlight.hash(&mut hasher);
    hasher.finish()
}

fn hash_preview<H: std::hash::Hasher>(
    hasher: &mut H,
    tag: u8,
    preview: Option<(&[Point3], Option<Point3>)>,
) {
    use std::hash::Hash;
    match preview {
        Some((points, cursor)) => {
            tag.hash(hasher);
            points.len().hash(hasher);
            for point in points {
                hash_point(hasher, *point);
            }
            match cursor {
                Some(point) => {
                    1u8.hash(hasher);
                    hash_point(hasher, point);
                }
                None => 0u8.hash(hasher),
            }
        }
        None => 255u8.hash(hasher),
    }
}

fn hash_point<H: std::hash::Hasher>(hasher: &mut H, point: Point3) {
    use std::hash::Hash;
    point.x.to_bits().hash(hasher);
    point.y.to_bits().hash(hasher);
    point.z.to_bits().hash(hasher);
}
