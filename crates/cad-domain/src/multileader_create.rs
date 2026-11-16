//! The MULTILEADER **creation** path: the two-vertex definition ADR 0019
//! decision F.2 derives from a captured arrow position, a landing point and the
//! annotation text, plus the modelled creation options.
//!
//! Split out of `multileader.rs` for the code-size gate. The rule itself is
//! unchanged: one root, one line, two vertices, and the display derived by the
//! single [`MultiLeaderGeometry::new`] builder — this module adds **no** second
//! derivation.

use crate::dimension_synthesis::{is_finite_point, sub};
use crate::multileader::{
    length3, MLeaderContent, MLeaderCreateOptions, MLeaderLine, MLeaderPathType, MLeaderReserved,
    MLeaderRoot, MLeaderStyleValues, MLeaderText, MultiLeaderGeometry, MultiLeaderParts,
    PlaneFrame, CREATED_TEXT_ATTACHMENT_POINT,
};
use crate::{CadError, CadResult, Point3};

impl MultiLeaderGeometry {
    /// Build a multileader from a **creation** capture: the arrow position, the
    /// landing point (the content connection point) and the annotation text
    /// (ADR 0019 decision F.2, P7c).
    ///
    /// The definition is one root, one line and **two vertices**, joined straight:
    ///
    /// * vertex 1 is the captured arrow position;
    /// * vertex 2 is the **dogleg start**, derived from the captured landing point
    ///   and the style's dogleg length: the dogleg runs horizontally toward the
    ///   text (the side the landing point lies on) for `dogleg_length`, so the
    ///   second vertex is `landing - (dogleg_length · side)` and the display's
    ///   dogleg (which starts at the last vertex and stops `landing_gap` short of
    ///   the connection point) is exactly that segment minus the gap;
    /// * the text sits at the landing point with the style's height and the
    ///   style's vertical attachment, centred horizontally (see
    ///   [`CREATED_TEXT_ATTACHMENT_POINT`]).
    ///
    /// Nothing is invented beyond those derivations: the typeface, sizes, gap,
    /// dogleg length and path type all come from the caller's resolved style
    /// snapshot (the `MLeaderStyle` table), and `override_mask` stays `0` because
    /// a created leader overrides none of its style's values.
    ///
    /// This is [`Self::created_with_options`] with every option left at its
    /// resolved-style default; the derivation itself lives there (one rule, not
    /// two).
    ///
    /// Refusals (each changes nothing; the caller opens no transaction):
    /// non-finite points, a pair that is not coplanar on one `+Z` plane, an empty
    /// text, a non-straight path type, a non-positive or non-finite style size,
    /// a landing point no farther than the style's dogleg length, and a
    /// degenerate leader segment.
    pub fn created(
        style: MLeaderStyleValues,
        style_name: String,
        left_attachment: i16,
        arrow: Point3,
        landing: Point3,
        text: String,
    ) -> CadResult<Self> {
        Self::created_with_options(
            style,
            style_name,
            left_attachment,
            arrow,
            landing,
            text,
            MLeaderCreateOptions::default(),
        )
    }

    /// [`Self::created`] with the modelled creation options (ADR 0019 F.2).
    ///
    /// `style_left_attachment` is the style's resolved vertical attachment and is
    /// what the leader stores unless [`MLeaderCreateOptions::left_attachment`]
    /// overrides it. Every other derivation is the one `created` rule:
    ///
    /// * the dogleg length is [`MLeaderCreateOptions::dogleg_length`] when set,
    ///   the style's `landing_distance` otherwise, and the second vertex is
    ///   `landing - direction · length`;
    /// * the dogleg direction is [`MLeaderCreateOptions::dogleg_angle`] when set
    ///   (the direction the dogleg travels toward the annotation), the arrow/text
    ///   side otherwise. It is stored verbatim as `LEADER 11`, so a reload reads
    ///   exactly the direction that was asked for;
    /// * the text carries the effective attachment pair (see
    ///   [`MLeaderCreateOptions`]).
    ///
    /// Refusals beyond [`Self::created`]'s (all named, nothing written):
    /// `mleader.option_dogleg` (non-finite or non-positive length),
    /// `mleader.option_angle` (non-finite angle), `mleader.option_attachment`
    /// (a horizontal code outside `1..=3` or a vertical code outside `0..=10`).
    pub fn created_with_options(
        style: MLeaderStyleValues,
        style_name: String,
        style_left_attachment: i16,
        arrow: Point3,
        landing: Point3,
        text: String,
        options: MLeaderCreateOptions,
    ) -> CadResult<Self> {
        if !is_finite_point(arrow) || !is_finite_point(landing) {
            return Err(CadError::InvalidInput(
                "mleader.points_not_finite: the arrow and landing points must be finite".into(),
            ));
        }
        let z_tolerance = 1e-9 * (1.0 + arrow.z.abs() + landing.z.abs());
        if (landing.z - arrow.z).abs() > z_tolerance {
            return Err(CadError::InvalidInput(
                "mleader.plane: the arrow and landing points must lie in one +Z plane".into(),
            ));
        }
        if text.trim().is_empty() {
            return Err(CadError::InvalidInput(
                "mleader.text_required: a created multileader needs a non-empty text".into(),
            ));
        }
        if style.path_type != MLeaderPathType::StraightLineSegments {
            return Err(CadError::Unsupported(
                "mleader.path_type: only straight-line leaders can be created; the resolved \
                 style is spline or invisible"
                    .into(),
            ));
        }
        let dogleg_length = match options.dogleg_length {
            Some(value) => {
                if !value.is_finite() || value <= 0.0 {
                    return Err(CadError::InvalidInput(format!(
                        "mleader.option_dogleg: the requested dogleg length must be finite and \
                         positive, got {value}"
                    )));
                }
                value
            }
            None => style.landing_distance,
        };
        if !dogleg_length.is_finite() || dogleg_length <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mleader.style_invalid: the dogleg length must be finite and positive, got \
                 {dogleg_length}"
            )));
        }
        if !style.text_height.is_finite() || style.text_height <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mleader.style_invalid: the text height must be finite and positive, got {}",
                style.text_height
            )));
        }
        if !style.arrowhead_size.is_finite() || style.arrowhead_size <= 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mleader.style_invalid: the arrowhead size must be finite and positive, got {}",
                style.arrowhead_size
            )));
        }
        if !style.landing_gap.is_finite() || style.landing_gap < 0.0 {
            return Err(CadError::InvalidInput(format!(
                "mleader.style_invalid: the landing gap must be finite and non-negative, got {}",
                style.landing_gap
            )));
        }
        // The text side: the dogleg points from the leader toward the landing
        // point along +X when the text is to the right, -X when it is to the
        // left. An exactly vertical leader (equal X) takes +X, the reading
        // direction, rather than inventing a side from the Y delta. An explicit
        // `dogleg_angle` replaces that derivation with the requested direction
        // (still the direction the dogleg travels toward the annotation).
        let (direction_x, direction_y) = match options.dogleg_angle {
            Some(angle) => {
                if !angle.is_finite() {
                    return Err(CadError::InvalidInput(format!(
                        "mleader.option_angle: the requested dogleg angle must be finite, got \
                         {angle}"
                    )));
                }
                // `sin_cos` is (sin, cos); the direction is (cos, sin).
                let (sin, cos) = angle.sin_cos();
                (cos, sin)
            }
            None => {
                let side = if landing.x < arrow.x { -1.0 } else { 1.0 };
                (side, 0.0)
            }
        };
        // The two attachment codes were preserved verbatim by the import path, so
        // the ranges below are the DXF codes the writer maps one-to-one (entity
        // `179`/`173`, `CONTEXT_DATA 171`/`174`).
        let attachment_point = match options.attachment_point {
            Some(value) => {
                if !(1..=3).contains(&value) {
                    return Err(CadError::InvalidInput(format!(
                        "mleader.option_attachment: the horizontal text attachment must be one of \
                         1 (left), 2 (centre) or 3 (right), got {value}"
                    )));
                }
                value
            }
            None => CREATED_TEXT_ATTACHMENT_POINT,
        };
        let left_attachment = match options.left_attachment {
            Some(value) => {
                if !(0..=10).contains(&value) {
                    return Err(CadError::InvalidInput(format!(
                        "mleader.option_attachment: the vertical text attachment must be one of \
                         the DXF codes 0..=10, got {value}"
                    )));
                }
                value
            }
            None => style_left_attachment,
        };
        let dogleg_start = Point3 {
            x: landing.x - direction_x * dogleg_length,
            y: landing.y - direction_y * dogleg_length,
            z: landing.z,
        };
        let span = length3(sub(arrow, landing));
        if span <= dogleg_length {
            return Err(CadError::InvalidInput(format!(
                "mleader.landing_too_close: the landing point must be farther from the arrow \
                 position than the style dogleg length {dogleg_length}, got {span}"
            )));
        }
        if length3(sub(arrow, dogleg_start)) <= 1e-12 {
            return Err(CadError::InvalidInput(
                "mleader.leader_degenerate: the arrow position and the derived dogleg start \
                 coincide"
                    .into(),
            ));
        }
        let height = style.text_height;
        let parts = MultiLeaderParts {
            style: style.clone(),
            style_name,
            override_mask: 0,
            content: MLeaderContent::Text,
            text: MLeaderText {
                string: text,
                location: landing,
                rotation: 0.0,
                height,
                width: 0.0,
                attachment_point,
                left_attachment,
            },
            roots: vec![MLeaderRoot {
                leader_index: 0,
                landing_point: landing,
                // The dogleg's own direction (start -> connection point); the
                // exporter writes it verbatim as LEADER group 11 and the import
                // reads it verbatim, so it is not re-derived on reload.
                dogleg_vector: Point3 {
                    x: direction_x,
                    y: direction_y,
                    z: 0.0,
                },
                dogleg_length,
            }],
            lines: vec![MLeaderLine {
                root_index: 0,
                index: 0,
                points: vec![arrow, dogleg_start],
                path_type: MLeaderPathType::StraightLineSegments,
                // No per-line overrides: the root/style values apply.
                override_flags: 0,
                arrowhead_size: style.arrowhead_size,
                arrowhead_handle: None,
            }],
            reserved: MLeaderReserved::default(),
            plane: PlaneFrame::default(),
        };
        Self::new(parts)
    }
}
