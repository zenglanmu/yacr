//! The `GRIPSTRETCH` fixture and the shared assertions of `tests::drawing::grip`.
//!
//! Split out of `grip.rs` so that file stays well under the 1000-line cap now that
//! every handle family (LINE, POINT, CIRCLE, ARC, ELLIPSE, POLYLINE, SPLINE) has
//! command-level tests. Everything here is fixture/plumbing only — no test lives in
//! this module, and every item is `pub(super)` so the sibling `grip` module sees
//! exactly the same surface it used to define itself.

use super::*;

use crate::grips::{GripId, GripKind};
use crate::GripStretchCommand;

pub(super) const LINE: u128 = 1;
pub(super) const POINT: u128 = 2;
pub(super) const CIRCLE: u128 = 3;
pub(super) const PAPER_LINE: u128 = 4;
/// The `ELLIPSE` whose axis/centre grips are dragged here (`Center 0`,
/// `Quadrant 0..=3` on a full ellipse).
pub(super) const ELLIPSE: u128 = 5;
/// The `ARC` whose endpoint grips are dragged here (`Center 0, Endpoint 0/1`).
pub(super) const ARC: u128 = 6;
/// The `POLYLINE` whose `Vertex` grips are dragged here.
pub(super) const POLYLINE: u128 = 7;
/// A geometry with **no** grip model at all (an `INSERT`: an instance transform is
/// not an entity handle), used for the `grip.unsupported_kind` refusal now that
/// `CIRCLE`, `ARC`, `ELLIPSE`, `POLYLINE` and `SPLINE` all have handles.
pub(super) const INSERT: u128 = 8;
/// The `SPLINE` whose `ControlPoint` grips are dragged here.
pub(super) const SPLINE: u128 = 9;
/// The `TEXT` whose `Insertion` grip is dragged here (`Insertion` `0`; MTEXT is
/// the same variant, so one text fixture covers both classes).
pub(super) const TEXT: u128 = 10;

/// The fixture text's stored insertion point: non-round coordinates, so "only
/// `position` moved" is a real bit comparison. Non-default alignment, a non-zero
/// rotation and a `wrap_width` make the "untouched field" assertions meaningful.
pub(super) const TEXT_AT: Point3 = Point3 {
    x: 1.0 / 3.0,
    y: -2.0 / 7.0,
    z: 0.5,
};
pub(super) const TEXT_CONTENT: &str = "GRIP\\Psecond line";
pub(super) const TEXT_HEIGHT: f64 = 1.125;
pub(super) const TEXT_ROTATION: f64 = 0.75;
pub(super) const TEXT_WRAP: f64 = 12.5;

/// The fixture ellipse: major semi-axis `(4, 0, 0)`, ratio `0.5` (minor 2), a
/// full turn. Non-round centre so "the untouched fields are bit-identical" is a
/// real comparison.
pub(super) const ELLIPSE_CENTER: Point3 = Point3 {
    x: 1.0,
    y: -0.5,
    z: 2.0,
};
pub(super) const ELLIPSE_MAJOR: Point3 = Point3 {
    x: 4.0,
    y: 0.0,
    z: 0.0,
};
pub(super) const ELLIPSE_RATIO: f64 = 0.5;

/// The stored `ARC` of the fixture: centre at the origin, radius 5, from angle 0
/// over 1.5 rad, so the two endpoint handles are easy to aim at.
pub(super) const ARC_START_ANGLE: f64 = 0.0;
pub(super) const ARC_SWEEP: f64 = 1.5;
pub(super) const ARC_RADIUS: f64 = 5.0;
/// The fixture polyline's stored bulges (non-zero, so "bulges untouched" is a
/// real comparison at the command level too).
pub(super) const POLYLINE_BULGES: [f64; 3] = [0.0, 0.25, 0.0];

/// Non-round coordinates so an "untouched endpoint" assertion is a real bit
/// comparison rather than a happy accident of decimal literals.
pub(super) const LINE_START: Point3 = Point3 {
    x: 0.1,
    y: 0.2,
    z: 1.0 / 3.0,
};
pub(super) const LINE_END: Point3 = Point3 {
    x: -4.5,
    y: 6.25,
    z: 0.5,
};
pub(super) const POINT_AT: Point3 = Point3 {
    x: 3.0,
    y: 4.0,
    z: 0.0,
};

/// A drawing with a `LINE` (id 1), a `POINT` (id 2), a `CIRCLE` (id 3), a
/// paper-space `LINE` (id 4, the non-addressable target), an `ELLIPSE` (id 5,
/// axis/centre drags), an `ARC` (id 6, endpoint drags), a `POLYLINE` (id 7, vertex
/// drags), an `INSERT` (id 8, the unsupported-kind target), a `SPLINE` (id 9,
/// control-point drags) and a `TEXT` (id 10, the insertion-point drag). Nothing is
/// created through a command, so the
/// undo stack starts empty and a drag's undo depth is unambiguous.
pub(super) fn grip_app() -> (Application, SessionState) {
    let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
    b.insert_layer(Layer {
        id: LayerId(0),
        name: "0".into(),
        visible: true,
    })
    .unwrap();
    b.insert_entity(ent(
        LINE,
        SemanticGeometry::Line {
            start: LINE_START,
            end: LINE_END,
        },
        0,
    ))
    .unwrap();
    b.insert_entity(ent(POINT, SemanticGeometry::Point(POINT_AT), 1))
        .unwrap();
    b.insert_entity(ent(
        CIRCLE,
        SemanticGeometry::Circle {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: 5.0,
        },
        2,
    ))
    .unwrap();
    let mut paper = ent(
        PAPER_LINE,
        SemanticGeometry::Line {
            start: p3(0.0, 0.0, 0.0),
            end: p3(1.0, 0.0, 0.0),
        },
        3,
    );
    paper.space = SpaceId::Paper(LayoutId(1));
    b.insert_entity(ent(
        ELLIPSE,
        SemanticGeometry::Ellipse {
            center: ELLIPSE_CENTER,
            normal: p3(0.0, 0.0, 1.0),
            major_axis: ELLIPSE_MAJOR,
            ratio: ELLIPSE_RATIO,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        },
        4,
    ))
    .unwrap();
    // The drawing must carry the layout the paper-space entity references.
    b.insert_layout(Layout {
        id: LayoutId(1),
        name: "Sheet1".into(),
        viewports: Vec::new(),
    })
    .unwrap();
    b.insert_entity(paper).unwrap();
    b.insert_entity(ent(
        ARC,
        SemanticGeometry::Arc {
            center: p3(0.0, 0.0, 0.0),
            normal: p3(0.0, 0.0, 1.0),
            radius: ARC_RADIUS,
            start: ARC_START_ANGLE,
            sweep: ARC_SWEEP,
        },
        5,
    ))
    .unwrap();
    b.insert_entity(ent(
        POLYLINE,
        SemanticGeometry::Polyline {
            points: vec![p3(0.0, 0.0, 1.0), p3(10.0, 0.0, 1.0), p3(10.0, 5.0, 1.0)],
            bulges: POLYLINE_BULGES.to_vec(),
            closed: false,
        },
        6,
    ))
    .unwrap();
    b.insert_entity(ent(
        INSERT,
        SemanticGeometry::Insert {
            block: BlockId(0),
            transform: Transform3::translation(p3(5.0, 5.0, 0.0)),
        },
        7,
    ))
    .unwrap();
    // A clamped cubic: 4 control points need `4 + 3 + 1 = 8` knots.
    b.insert_entity(ent(
        SPLINE,
        SemanticGeometry::Spline {
            degree: 3,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![
                p3(0.0, 0.0, 1.0),
                p3(1.0, 2.0, 1.0),
                p3(3.0, 2.0, 1.0),
                p3(4.0, 0.0, 1.0),
            ],
            weights: vec![1.0; 4],
        },
        8,
    ))
    .unwrap();
    // The text fixture: `AcDbText`-classed (the grip model reads the geometry only;
    // TEXT and MTEXT share the variant) with a `wrap_width`, a non-zero rotation
    // and a centre/middle attachment, so "every other field is bit-identical" is a
    // real claim at the command level too.
    let text = ent(
        TEXT,
        SemanticGeometry::Text {
            text: TEXT_CONTENT.into(),
            position: TEXT_AT,
            style: StyleId(0),
            height: TEXT_HEIGHT,
            rotation: TEXT_ROTATION,
            font: Some("arial.ttf".into()),
            h_align: TextAlignH::Center,
            v_align: TextAlignV::Middle,
            wrap_width: Some(TEXT_WRAP),
        },
        9,
    );
    b.insert_entity(text).unwrap();
    let drawing = b.finish().unwrap();

    let mut app = Application::new();
    app.workspace.documents.insert(
        DocumentId(1),
        Document {
            id: DocumentId(1),
            drawing: Arc::new(drawing),
            identity: DocumentIdentity::Sha256([11u8; 32]),
            units: UnitContext::drawing_units(),
            resource_keys: Vec::new(),
        },
    );
    app.workspace.viewports.insert(
        ViewportId(1),
        Viewport::new(ViewportId(1), DocumentId(1), [800.0, 600.0]),
    );
    (app, SessionState::new(DocumentId(1), AppMode::Work))
}

pub(super) fn grip_id(entity: u128, kind: GripKind, index: u8) -> GripId {
    GripId {
        entity: EntityId(entity),
        kind,
        index,
    }
}

/// The payload a grip gesture would produce: one handle id plus the world point.
pub(super) fn drag(entity: u128, kind: GripKind, index: u8, to: Point3) -> CommandPayload {
    CommandPayload::GripStretch(Box::new(GripStretchCommand {
        id: grip_id(entity, kind, index),
        to,
    }))
}

pub(super) fn undo_depth(app: &Application) -> usize {
    app.history
        .get(&DocumentId(1))
        .map(|history| history.undo_depth())
        .unwrap_or(0)
}

pub(super) fn geometry_of(app: &Application, entity: u128) -> SemanticGeometry {
    app.workspace.documents[&DocumentId(1)]
        .drawing
        .entity(EntityId(entity))
        .unwrap_or_else(|| panic!("entity {entity} is gone"))
        .geometry
        .clone()
}

pub(super) fn endpoints_of(app: &Application, entity: u128) -> (Point3, Point3) {
    match geometry_of(app, entity) {
        SemanticGeometry::Line { start, end } => (start, end),
        other => panic!("entity {entity} is not a LINE: {other:?}"),
    }
}

/// The documented exact midpoint of two stored points (`(start + end) * 0.5`),
/// recomputed here independently of the model.
pub(super) fn midpoint_of(start: Point3, end: Point3) -> Point3 {
    Point3 {
        x: (start.x + end.x) * 0.5,
        y: (start.y + end.y) * 0.5,
        z: (start.z + end.z) * 0.5,
    }
}

pub(super) fn position_of(app: &Application, entity: u128) -> Point3 {
    match geometry_of(app, entity) {
        SemanticGeometry::Point(position) => position,
        other => panic!("entity {entity} is not a POINT: {other:?}"),
    }
}

/// The stored `ARC` fields of `entity` (`center, normal, radius, start, sweep`).
pub(super) fn arc_parts(app: &Application, entity: u128) -> (Point3, Point3, f64, f64, f64) {
    match geometry_of(app, entity) {
        SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } => (center, normal, radius, start, sweep),
        other => panic!("entity {entity} is not an ARC: {other:?}"),
    }
}

/// The stored `POLYLINE` fields of `entity` (`points, bulges, closed`).
pub(super) fn polyline_parts(app: &Application, entity: u128) -> (Vec<Point3>, Vec<f64>, bool) {
    match geometry_of(app, entity) {
        SemanticGeometry::Polyline {
            points,
            bulges,
            closed,
        } => (points, bulges, closed),
        other => panic!("entity {entity} is not a POLYLINE: {other:?}"),
    }
}

/// The Euclidean length of a vector (used to read a stored semi-axis).
pub(super) fn length_of(v: Point3) -> f64 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// The stored `ELLIPSE` fields of `entity`
/// (`center, normal, major_axis, ratio, start, sweep`).
pub(super) fn ellipse_parts(
    app: &Application,
    entity: u128,
) -> (Point3, Point3, Point3, f64, f64, f64) {
    match geometry_of(app, entity) {
        SemanticGeometry::Ellipse {
            center,
            normal,
            major_axis,
            ratio,
            start,
            sweep,
        } => (center, normal, major_axis, ratio, start, sweep),
        other => panic!("entity {entity} is not an ELLIPSE: {other:?}"),
    }
}

/// The stored `TEXT` fields of `entity` that a drag must leave alone
/// (`text, style, height, rotation, font, h_align, v_align, wrap_width`), plus the
/// insertion point — so a caller can assert the position and compare the rest.
#[allow(clippy::type_complexity)]
pub(super) fn text_parts(
    app: &Application,
    entity: u128,
) -> (
    String,
    Point3,
    StyleId,
    f64,
    f64,
    Option<String>,
    TextAlignH,
    TextAlignV,
    Option<f64>,
) {
    match geometry_of(app, entity) {
        SemanticGeometry::Text {
            text,
            position,
            style,
            height,
            rotation,
            font,
            h_align,
            v_align,
            wrap_width,
        } => (
            text, position, style, height, rotation, font, h_align, v_align, wrap_width,
        ),
        other => panic!("entity {entity} is not a TEXT: {other:?}"),
    }
}

/// Assert a refusal with the exact stable reason code, and that the refusal
/// variant is the documented one (`InvalidInput` for its family,
/// `Unsupported` for the kind/addressability refusals).
pub(super) fn refused(error: CadError, code: &str, unsupported: bool) -> String {
    let message = match (error, unsupported) {
        (CadError::Unsupported(message), true) | (CadError::InvalidInput(message), false) => {
            message
        }
        (other, _) => panic!("unexpected refusal variant for {code}: {other:?}"),
    };
    assert!(
        message.starts_with(code),
        "reason must start with {code}, got {message}"
    );
    message
}

/// Every refusal must leave the drawing and the undo stack exactly as they were.
pub(super) fn assert_untouched(app: &Application, revision: Revision, undo: usize) {
    assert_eq!(
        revision_of(app),
        revision,
        "a refusal must not bump revision"
    );
    assert_eq!(undo_depth(app), undo, "a refusal must not record history");
}

pub(super) fn revision_of(app: &Application) -> Revision {
    app.workspace.documents[&DocumentId(1)].drawing.revision()
}
