//! The typed `HATCHEDIT` payload data, split out of `payload_extra.rs` to keep
//! every code file inside the 1000-line gate.

use cad_domain::{HatchPatternType, HatchStyle};

/// Where one `HATCHEDIT DRAWORDER` option places the selected hatches.
///
/// AutoCAD's third choice (`Boundary`) needs a boundary/associativity record this
/// build does not store, so it is a named refusal rather than a silent
/// approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatchDrawOrder {
    /// Behind every non-hatch object in model space.
    BehindAll,
    /// In front of every other object in model space.
    InFrontOfAll,
    /// Behind its own boundary object (needs a boundary record — refused).
    BehindBoundary,
}

/// The typed option of the AutoCAD 2026 `HATCHEDIT` command.
///
/// The command line carries the option (`HATCHEDIT ANGLE 45`,
/// `HATCHEDIT STYLE OUTER`, `HATCHEDIT COLOR 30`, …) because this build has no
/// in-command dialog. The selection (one or more directly addressable model-space
/// hatches) is the rest of the input. Full contract:
/// [`crate::command_id::hatchedit`].
#[derive(Debug, Clone, PartialEq)]
pub enum HatchEditOption {
    /// `ANGLE <degrees>`: set the stored pattern angle (only a `Pattern` fill).
    Angle(f64),
    /// `SCALE <factor>`: set the stored pattern scale (> 0, only a `Pattern`
    /// fill).
    Scale(f64),
    /// `PATTERNTYPE USER|PREDEFINED|CUSTOM`: set DXF group 76.
    PatternType(HatchPatternType),
    /// `STYLE IGNORE|OUTER|NORMAL`: set the island-detection style (group 75).
    Style(HatchStyle),
    /// `DRAWORDER BEHIND|FRONT|BOUNDARY`: rewrite the stored draw order.
    DrawOrder(HatchDrawOrder),
    /// `SOLID`: replace the fill with the solid variant.
    Solid,
    /// `GRADIENT`: replace the fill with the default two-colour linear ramp.
    Gradient,
    /// `RECREATEBOUNDARY`: one closed polyline per stored loop.
    RecreateBoundary,
    /// `SEPARATE`: split a multi-loop hatch into one hatch per loop.
    SeparateHatches,
    /// `COLOR <aci>`: set an explicit ACI display colour (1..=255).
    Color(u8),
    /// `COLOR BYLAYER`: restore the symbolic ByLayer colour.
    ColorByLayer,
    /// `TRANSPARENCY <percent>`: set an explicit opacity (0..=90 percent
    /// transparent).
    Transparency(u8),
    /// `TRANSPARENCY BYLAYER`: restore the symbolic ByLayer transparency.
    TransparencyByLayer,
    /// `LAYER <name>`: move every selected hatch to that drawing layer.
    Layer(String),
    /// `ORIGIN`: named refusal — the domain stores no pattern origin.
    Origin,
    /// `ASSOCIATE`: named refusal — there is no associativity model.
    Associate,
    /// `DISASSOCIATE`: named refusal — dimensions/hatches are non-associative.
    Disassociate,
    /// `ANNOTATIVE`: named refusal — there is no annotative-scale model.
    Annotative,
    /// `ADDBOUNDARIES`: append the other selected closed entities as boundary
    /// loops (the selection holds exactly one hatch plus the sources).
    AddBoundaries,
    /// `REMOVEBOUNDARIES`: remove the stored loops matching the other selected
    /// closed entities (points + bulges, `1e-9`).
    RemoveBoundaries,
}
