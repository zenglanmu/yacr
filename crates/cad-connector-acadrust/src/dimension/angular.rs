//! Arc-length and large-radius (jogged) dimension synthesis.
//!
//! Submodule of [`crate::dimension`]. These two subtypes are not in the ADR
//! 0017 kind set (A.1/A.3), so they keep the pre-P5a compound synthesis; the
//! shared helpers ([`line_between`], [`arrowhead`], [`readable_angle`]) live in
//! `cad-domain` and are reused here.

use super::*;

/// Arc-length dimension: the measured arc plus extension lines, arrows and the
/// optional leader between `first_leader_point` and `second_leader_point`.
#[allow(clippy::too_many_arguments)]
pub(super) fn arc_length_geometry(
    center: Point3,
    first_extension: Point3,
    second_extension: Point3,
    start: f64,
    end: f64,
    normal: Point3,
    style: &DimensionStyleValues,
    leader: Option<(Point3, Point3)>,
) -> Option<SynthesizedDimension> {
    let radius = cad_geometry::distance(center, first_extension);
    if !cad_geometry::is_finite(center) || radius < 1e-9 {
        return None;
    }
    let sweep = normalize_sweep(end - start);
    let dir_at = |angle: f64| Point3 {
        x: angle.cos(),
        y: angle.sin(),
        z: 0.0,
    };
    let mut children = vec![SemanticGeometry::Arc {
        center,
        normal,
        radius,
        start,
        sweep,
    }];
    let end1 = cad_geometry::add(center, cad_geometry::scale(dir_at(start), radius));
    let end2 = cad_geometry::add(center, cad_geometry::scale(dir_at(start + sweep), radius));
    for (point, foot) in [(first_extension, end1), (second_extension, end2)] {
        if let Some(line) = line_between(point, foot) {
            children.push(line);
        }
    }
    if let Some((first, second)) = leader {
        if cad_geometry::is_finite(first) && cad_geometry::is_finite(second) {
            if let Some(line) = line_between(first, second) {
                children.push(line);
            }
        }
    }
    if style.arrow_size > 1e-9 {
        let sign = if sweep >= 0.0 { -1.0 } else { 1.0 };
        let t1 = Point3 {
            x: -dir_at(start).y,
            y: dir_at(start).x,
            z: 0.0,
        };
        let t2 = Point3 {
            x: -dir_at(start + sweep).y,
            y: dir_at(start + sweep).x,
            z: 0.0,
        };
        children.push(arrowhead(
            end1,
            cad_geometry::scale(t1, sign),
            style.arrow_size,
        ));
        children.push(arrowhead(
            end2,
            cad_geometry::scale(t2, -sign),
            style.arrow_size,
        ));
    }
    let mid = dir_at(start + sweep / 2.0);
    Some(SynthesizedDimension {
        children,
        measurement: radius * sweep.abs(),
        text_position: cad_geometry::add(
            center,
            cad_geometry::scale(mid, radius + style.text_gap + style.text_height * 0.5),
        ),
        text_rotation: readable_angle(start + sweep / 2.0),
    })
}

/// Jogged / large-radius radial dimension: the radial leader, the jog segment
/// oriented by `jog_angle`, and the connector that closes the remaining gap to
/// the chord point.
///
/// The jog leaves the jog point along `jog_angle` for the chord distance; a
/// connector then reaches the chord point. When `jog_angle` already points at
/// the chord the two segments collapse into one, matching the un-jogged case.
/// AutoCAD's true zigzag jog is still only approximated, so this stays `Partial`.
pub(super) fn large_radial_geometry(
    center: Point3,
    chord: Point3,
    jog: Point3,
    jog_angle: f64,
) -> Option<SynthesizedDimension> {
    if !cad_geometry::is_finite(center)
        || !cad_geometry::is_finite(chord)
        || !cad_geometry::is_finite(jog)
    {
        return None;
    }
    let mut children = Vec::new();
    let chord_length = cad_geometry::distance(jog, chord);
    let jog_end = if jog_angle.is_finite() && chord_length > 1e-9 {
        cad_geometry::add(
            jog,
            cad_geometry::scale(
                Point3 {
                    x: jog_angle.cos(),
                    y: jog_angle.sin(),
                    z: 0.0,
                },
                chord_length,
            ),
        )
    } else {
        chord
    };
    if let Some(line) = line_between(jog, jog_end) {
        children.push(line);
    }
    if let Some(line) = line_between(jog_end, chord) {
        children.push(line);
    }
    if let Some(line) = line_between(center, jog) {
        children.push(line);
    }
    Some(SynthesizedDimension {
        children,
        measurement: cad_geometry::distance(center, chord),
        text_position: jog,
        text_rotation: 0.0,
    })
}
