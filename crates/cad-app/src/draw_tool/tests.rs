//! Draw-tool state-machine contracts.
//!
//! The scenario groups live in sibling submodules (`arc`, `capture`, `picks`,
//! `shape_intents`, `transform`, plus the pre-existing groups) to keep this file
//! under the code-size limit. The shared fixtures (`p`, `pick`) and the
//! canonical arity contract stay here.

use super::*;
use cad_domain::{DocumentId, EntityId, InstancePath, SelectionRef};

fn p(x: f64, y: f64) -> Point3 {
    Point3 { x, y, z: 0.0 }
}

/// A synthetic ref for the draw-tool-level tests (identity only; the tool never
/// reads the database).
fn pick(id: u128) -> SelectionRef {
    SelectionRef {
        document: DocumentId(1),
        entity: EntityId(id),
        instance: InstancePath::default(),
        sub_element: None,
    }
}

#[test]
fn required_points_and_selection_are_explicit_per_kind() {
    for kind in DrawToolKind::ALL {
        let expected = match kind {
            DrawToolKind::Arc
            | DrawToolKind::Ellipse
            | DrawToolKind::Scale
            | DrawToolKind::DimLinear
            | DrawToolKind::DimAligned
            // DIMROTATED is the same three picks as DIMLINEAR.
            | DrawToolKind::DimRotated
            // CIRCLE 3P is the three-point circumcircle.
            | DrawToolKind::Circle3P
            | DrawToolKind::DimAngular => 3,
            DrawToolKind::Spline | DrawToolKind::Align | DrawToolKind::Stretch => 4,
            DrawToolKind::Offset
            | DrawToolKind::Point
            | DrawToolKind::ArrayPolar
            | DrawToolKind::DimCenter
            // TEXT captures one insertion point; its text is a typed string, not a
            // point (ADR 0019 decision A.1). TEXTEDIT's one step is its entity
            // pick (ADR 0020 amendment A).
            | DrawToolKind::Text
            | DrawToolKind::TextEdit
            // COPYBASE/CUTBASE capture their base point; PASTECLIP its insertion
            // point. PASTEBLOCK has the same one-point shape.
            | DrawToolKind::CopyBase
            | DrawToolKind::CutBase
            | DrawToolKind::PasteClip
            | DrawToolKind::PasteBlock
            // HATCHPOINT/BOUNDARY capture their one internal point.
            | DrawToolKind::HatchPoint
            | DrawToolKind::Boundary
            // INSERT captures its one insertion point; the block name is the
            // terminal typed text. BLOCK captures its one base point; its name is
            // also the terminal typed text.
            | DrawToolKind::InsertBlock
            | DrawToolKind::BlockDefine => 1,
            // WIPEOUT is the open-ended polygon capture: three vertices are the
            // minimum a closed ring needs.
            DrawToolKind::Wipeout => 3,
            // ARRAYRECT captures no points: it is defined entirely by the session
            // typed parameters plus the selection snapshot. HATCH has the same
            // pre-selection shape: the selected entities are the boundary and the
            // pattern parameters are the typed channel. DIVIDE/LENGTHEN are the
            // same zero-point pre-selection pair (the one selected curve is the
            // input and their settings are typed parameters).
            DrawToolKind::ArrayRect
            | DrawToolKind::Hatch
            | DrawToolKind::Divide
            | DrawToolKind::Lengthen
            | DrawToolKind::ScaleText => 0,
            _ => 2,
        };
        assert_eq!(kind.required_points(), expected, "{kind:?}");
    }
    // The arity model: PLINE/SPLINE, the baseline/continue pair and WIPEOUT are
    // open-ended.
    assert_eq!(DrawToolKind::Polyline.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::Spline.arity(), PointArity::AtLeast(4));
    assert_eq!(DrawToolKind::DimBaseline.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::DimContinue.arity(), PointArity::AtLeast(2));
    assert_eq!(DrawToolKind::Wipeout.arity(), PointArity::AtLeast(3));
    assert_eq!(DrawToolKind::Point.arity(), PointArity::Exact(1));
    for kind in DrawToolKind::ALL {
        if kind.arity().is_open_ended() {
            continue;
        }
        assert_eq!(
            kind.arity(),
            PointArity::Exact(kind.required_points()),
            "{kind:?} must stay fixed-arity"
        );
    }
    assert!(PointArity::AtLeast(2).is_open_ended());
    assert!(!PointArity::Exact(2).is_open_ended());
    assert_eq!(PointArity::AtLeast(2).minimum(), 2);
    assert_eq!(PointArity::Exact(3).minimum(), 3);
    assert!(!DrawToolKind::Line.requires_selection());
    assert!(!DrawToolKind::Circle.requires_selection());
    assert!(!DrawToolKind::Arc.requires_selection());
    assert!(!DrawToolKind::Polyline.requires_selection());
    assert!(!DrawToolKind::Polyline.needs_picked_entities());
    assert!(DrawToolKind::Move.requires_selection());
    assert!(!DrawToolKind::Trim.requires_selection());
    assert!(DrawToolKind::Copy.requires_selection());
    assert!(!DrawToolKind::Extend.requires_selection());
    assert!(DrawToolKind::Rotate.requires_selection());
    assert!(DrawToolKind::Scale.requires_selection());
    assert!(DrawToolKind::Mirror.requires_selection());
    assert!(DrawToolKind::Offset.requires_selection());
    // STRETCH is Exact(4) with no entity picks; the window itself defines the set.
    assert_eq!(DrawToolKind::Stretch.arity(), PointArity::Exact(4));
    assert!(!DrawToolKind::Stretch.requires_selection());
    assert!(!DrawToolKind::Stretch.needs_picked_entities());
}

mod arc;
mod array;
mod capture;
mod clipboard;
mod dimension;
mod draw_variants;
mod hatch;
mod keys;
mod mleader;
mod mtext;
mod picks;
mod point_spline;
mod shape_intents;
mod text;
mod transform;
