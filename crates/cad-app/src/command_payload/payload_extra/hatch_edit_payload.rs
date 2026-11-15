//! The typed `HATCHEDIT` payload data, split out of `payload_extra.rs` to keep
//! every code file inside the 1000-line gate.

use cad_domain::{HatchPatternType, HatchStyle};
use std::sync::Arc;

/// The `.pat` library text a host read delivered for `PATTERNFILE`.
///
/// The command line is parsed in the session, which cannot read a filesystem
/// (it is also compiled to wasm), so the host strategy reads the file and the
/// text travels in the payload — exactly like an import request carries the bytes
/// the host read. `Debug` prints the size only: the option is echoed into
/// diagnostics, and a whole pattern library does not belong there.
#[derive(Clone, PartialEq, Eq)]
pub struct PatternFileText(Arc<str>);

impl PatternFileText {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PatternFileText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PatternFileText(<{} bytes>)", self.0.len())
    }
}

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
    /// `PATTERN <name>`: replace the fill with the named built-in `.pat`
    /// library pattern, keeping the hatch's stored pattern angle/scale/`double`
    /// and recording the fill as a predefined pattern (`76=1`).
    ///
    /// A pattern fill is required (a solid/gradient fill is
    /// `hatchedit.pattern_requires_pattern`), the name is matched
    /// case-insensitively against the one library table, and an unknown name is
    /// `hatchedit.unknown_pattern`. A user `acad.pat`/`acadiso.pat` entry is not
    /// selectable through this form: use [`Self::PatternFile`].
    Pattern(String),
    /// `PATTERNFILE <path.pat> [<name>]`: apply a pattern from a user `.pat`
    /// file, keeping the hatch's stored angle/scale/`double` and recording the
    /// fill as a custom pattern (`76=2`, what AutoCAD writes for a user-file
    /// pattern).
    ///
    /// `text` is the file content the **host** read
    /// (`cad_host::HostResources::read_named_file`): this command never touches
    /// the filesystem. `None` means the payload was built without that read and
    /// is refused as `hatchedit.pattern_file_not_read`, never treated as an empty
    /// file. With no `name` the file must hold exactly one pattern
    /// (`hatchedit.pattern_file_ambiguous` otherwise); with a name the file is
    /// searched first and the built-in library second, and a name in neither is
    /// `hatchedit.unknown_pattern` naming both sources.
    PatternFile {
        path: String,
        name: Option<String>,
        text: Option<PatternFileText>,
    },
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
    /// `ORIGIN <x> <y>`: move the pattern origin (the phase displacement of the
    /// pattern's definition rows, see `cad_domain::HatchGeometry`).
    ///
    /// Only a [`Pattern`](Self::Pattern) fill draws rows, so a solid/gradient
    /// fill is `hatchedit.not_pattern`, exactly like `ANGLE`/`SCALE`.
    Origin { x: f64, y: f64 },
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
