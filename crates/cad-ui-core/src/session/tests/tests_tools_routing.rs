//! The draw-selector routing table and its one routing contract.
//!
//! Split out of `tests_tools.rs` (which is at the code-size limit) so the table
//! can grow without pushing that file over 1000 lines. The contract is unchanged:
//! every selector in the table must start the `cad-app` draw kind with the
//! matching machine key, capture no point yet and not be ready.

use super::super::*;
use super::test_support::*;

/// The draw selectors mapped to their `cad-app` machine keys.
///
/// ALIGN/STRETCH have their own dedicated tests; this table covers the rest.
const DRAW_SELECTORS: &[(&str, &str)] = &[
    ("LINE", "draw.line"),
    ("CIRCLE", "draw.circle"),
    // The three CIRCLE construction options are their own multi-word selectors and
    // their own captures (2P: diameter endpoints, 3P: three points, D: centre +
    // diameter point).
    ("CIRCLE 2P", "draw.circle2p"),
    ("CIRCLE 3P", "draw.circle3p"),
    ("CIRCLE D", "draw.circlediameter"),
    ("ARC", "draw.arc"),
    ("PLINE", "draw.polyline"),
    ("RECTANG", "draw.rectang"),
    ("ELLIPSE", "draw.ellipse"),
    ("POLYGON", "draw.polygon"),
    ("POINT", "draw.point"),
    ("SPLINE", "draw.spline"),
    ("TEXT", "draw.text"),
    ("MTEXT", "draw.mtext"),
    ("MLEADER", "draw.mleader"),
    ("MOVE", "draw.move"),
    ("TRIM", "draw.trim"),
    ("COPY", "draw.copy"),
    ("EXTEND", "draw.extend"),
    ("ROTATE", "draw.rotate"),
    ("SCALE", "draw.scale"),
    ("MIRROR", "draw.mirror"),
    ("OFFSET", "draw.offset"),
    ("CHAMFER", "draw.chamfer"),
    ("FILLET", "draw.fillet"),
    ("ARRAYRECT", "draw.arrayrect"),
    ("ARRAYPOLAR", "draw.arraypolar"),
    ("DIMLINEAR", "draw.dimlinear"),
    ("DIMALIGNED", "draw.dimaligned"),
    // DIMROTATED shares the three-pick shape; the angle is the session parameter,
    // so the capture starts with no point and is not ready.
    ("DIMROTATED", "draw.dimrotated"),
    ("DIMRADIUS", "draw.dimradius"),
    ("DIMDIAMETER", "draw.dimdiameter"),
    ("DIMCENTER", "draw.dimcenter"),
    ("DIMANGULAR", "draw.dimangular"),
    ("DIMORDINATE", "draw.dimordinate"),
    ("HATCH", "draw.hatch"),
    // HATCHPOINT captures one internal point (no selection; the boundary is
    // discovered from the point).
    ("HATCHPOINT", "draw.hatchpoint"),
    // The clipboard cluster's capture tools (`PASTECLIP` is insertion-point based
    // and `COPYBASE`/`CUTBASE` capture a base point over the selection).
    ("COPYBASE", "draw.copybase"),
    ("CUTBASE", "draw.cutbase"),
    ("PASTECLIP", "draw.pasteclip"),
    ("PASTEBLOCK", "draw.pasteblock"),
    // DIVIDE/LENGTHEN are the zero-point pre-selection edits: the one selected
    // curve is the whole input, so they start with no captured point and are not
    // ready until a selection exists.
    ("DIVIDE", "draw.divide"),
    ("LENGTHEN", "draw.lengthen"),
    // SCALETEXT is the zero-point pre-selection text edit: the selected text
    // entities are the whole input and the factor is the typed parameter channel.
    ("SCALETEXT", "draw.scaletext"),
    // INSERT is point-then-name: one insertion point, then the existing block
    // name through the terminal text channel (Work-only, no block-table write).
    ("INSERT", "draw.insertblock"),
    // BLOCK is selection-then-base-then-name: define a block from the selection.
    ("BLOCK", "draw.blockdefine"),
    // XLINE/RAY are the two-point construction-line pair (base, then a through
    // point). Both are Work-only and store one finite clipped segment.
    ("XLINE", "draw.xline"),
    ("RAY", "draw.ray"),
    // BREAKATPOINT is its own capture: entity pick, then a projected break point
    // (not BREAK's "first click is also the first break point" shape).
    ("BREAKATPOINT", "draw.breakatpoint"),
];

fn fresh_session() -> CadSession {
    session(Rc::new(FakeFiles {
        events: RefCell::new(Vec::new()),
    }))
}

#[test]
fn command_routing_starts_every_draw_tool_with_its_cad_app_key() {
    for (selector, key) in DRAW_SELECTORS {
        let session = fresh_session();
        session
            .command(selector)
            .unwrap_or_else(|error| panic!("{selector} refused: {error}"));
        let tool = session
            .snapshot(None, None, 0)
            .tool
            .unwrap_or_else(|| panic!("{selector} started no tool"));
        assert_eq!(&tool.key, key, "{selector}");
        assert_eq!(tool.points, 0, "{selector} starts with no captured point");
        assert!(!tool.ready, "{selector} must not be ready before any point");
    }
}
