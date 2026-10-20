//! First-class MULTILEADER entity: the resolved style snapshot, the leader
//! roots/lines, the opaque `reserved` bag and the derived display geometry
//! (ADR 0019 decision C).
//!
//! [`MultiLeaderGeometry`] is the single builder of a multileader's display: it
//! is derived from `style + text + roots + lines` by
//! [`MultiLeaderGeometry::build_display`], and the only writers are
//! [`MultiLeaderGeometry::new`] (creation path), [`MultiLeaderGeometry::from_import`]
//! (import path, which degrades honestly and never rejects) and
//! [`MultiLeaderGeometry::transformed`]. This mirrors `HatchGeometry`, so a
//! caller cannot store a display that disagrees with the definition.
//!
//! The display derivation is a **verbatim move** of the pre-P7b importer's
//! `cad-import-acadrust::dimension::multileader` synthesis: the same leader
//! paths (polyline or spline), the same arrowhead triangle, the same dogleg
//! formula and the same text placement. The acceptance for P7b-1 is display
//! equivalence with that `Compound` output, not a new rendering.

use crate::dimension_synthesis::{
    add, arrowhead, is_finite_point, length, line_between, normalize, scale, sub,
};
use crate::{
    CadError, CadResult, Point3, SemanticGeometry, StyleId, TextAlignH, TextAlignV, Transform3,
};

// ---------------------------------------------------------------------------
// Enums mirrored from DXF (ADR 0019 C).
// ---------------------------------------------------------------------------

/// DXF group `170`/line `170`: the leader path style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MLeaderPathType {
    /// `0`: no leader line is drawn.
    Invisible,
    /// `1`: straight line segments (a polyline through the vertices).
    #[default]
    StraightLineSegments,
    /// `2`: a smooth spline through the vertices.
    Spline,
}

/// Content kind of a migrated multileader.
///
/// ADR 0019 C.3 migrates only MText content (`172 = 2`) and content-less
/// leaders (`172 = 0`). Block and tolerance content deliberately stay on the
/// pre-P7b `Compound` path, so they never reach this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MLeaderContent {
    /// `172 = 0`: leaders without annotation content.
    #[default]
    None,
    /// `172 = 2`: the string lives in [`MultiLeaderText`].
    Text,
}

/// A plane frame: the DXF `110`/`111`/`112` base point, direction and vertical,
/// plus group `297` (normal reversed).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneFrame {
    pub origin: Point3,
    pub u: Point3,
    pub v: Point3,
    pub normal_reversed: bool,
}

impl Default for PlaneFrame {
    fn default() -> Self {
        PlaneFrame {
            origin: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            u: Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            v: Point3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
            normal_reversed: false,
        }
    }
}

/// The resolved MLEADERSTYLE/entity values the display consumes (ADR 0019 C/D).
///
/// This is a snapshot: editing the style object later does not retroactively
/// change an existing multileader. Every field drives the display or the text
/// style, so storing one without honouring it is not possible by construction.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderStyleValues {
    /// Effective path type (`170`/style `173`, override mask applied).
    pub path_type: MLeaderPathType,
    /// Effective "enable landing" (kept for round-trip; the display has no
    /// separate landing primitive).
    pub enable_landing: bool,
    /// Effective "enable dogleg" (the dogleg is the landing/dogleg segment).
    pub enable_dogleg: bool,
    /// Effective default dogleg length when a root carries no usable landing
    /// distance (style `43` / entity `41`).
    pub landing_distance: f64,
    /// Effective landing gap (style `42` / context `145`).
    pub landing_gap: f64,
    /// Effective arrowhead size (style `44` / entity `42`).
    pub arrowhead_size: f64,
    /// Effective text height (context `41` / entity `45`, scaled).
    pub text_height: f64,
    /// The text style identity the content `Text` carries.
    pub text_style: StyleId,
    /// Resolved text style name (DXF group `343` reference target).
    pub text_style_name: String,
    /// Font resource key of the text style, when known.
    pub text_font: Option<String>,
    /// Entity scale factor (DXF tail `45`).
    pub scale_factor: f64,
}

/// The annotation text of a migrated multileader.
///
/// The raw attachment codes are preserved verbatim; [`Self::h_align`] and
/// [`Self::v_align`] are the 3x3 alignment mapping the display uses (ADR 0019
/// U-3: the mapping is unverified against AutoCAD).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderText {
    pub string: String,
    pub location: Point3,
    pub rotation: f64,
    pub height: f64,
    /// DXF text width boundary (`43`).
    pub width: f64,
    /// DXF attachment point (`1` left, `2` center, `3` right).
    pub attachment_point: i16,
    /// DXF left attachment (`0`..`10`; the top/bottom position of the text).
    pub left_attachment: i16,
}

impl MLeaderText {
    /// The horizontal text alignment (1/2/3 -> left/center/right).
    pub fn h_align(&self) -> TextAlignH {
        match self.attachment_point {
            1 => TextAlignH::Left,
            3 => TextAlignH::Right,
            _ => TextAlignH::Center,
        }
    }

    /// The vertical text alignment (top/middle/bottom line positions).
    pub fn v_align(&self) -> TextAlignV {
        match self.left_attachment {
            0 | 1 => TextAlignV::Top,
            4..=8 => TextAlignV::Bottom,
            _ => TextAlignV::Middle,
        }
    }
}

/// One leader root (DXF `LEADER{}`).
///
/// `dogleg_vector` is the source direction from the connection point; it is the
/// fallback direction of the dogleg when the last vertex coincides with the
/// landing point. `dogleg_length` is the root's own landing distance.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderRoot {
    /// DXF group `90` inside `LEADER{}`.
    pub leader_index: i32,
    /// The content connection point (landing point), DXF group `10`.
    pub landing_point: Point3,
    /// The direction from the connection point, DXF group `11`.
    pub dogleg_vector: Point3,
    /// The root's landing/dogleg distance, DXF group `40`.
    pub dogleg_length: f64,
}

/// One leader line (DXF `LEADER_LINE{}`).
///
/// `root_index` is structural (the source nests lines inside a root); it is not
/// a DXF group. The per-line override flags and arrowhead fields are preserved
/// verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderLine {
    /// Index into [`MultiLeaderGeometry::roots`].
    pub root_index: usize,
    /// DXF group `91`: the line index inside its root.
    pub index: i32,
    /// Leader vertices, in source order (arrowhead end first).
    pub points: Vec<Point3>,
    /// DXF line `170`: the line's own path type (effective only when
    /// [`Self::PATH_TYPE`] is set in `override_flags`).
    pub path_type: MLeaderPathType,
    /// DXF group `93`: the per-line override bit word, verbatim.
    pub override_flags: u32,
    /// DXF line `40`: the line's arrowhead size.
    pub arrowhead_size: f64,
    /// DXF line `341`: a custom arrowhead block handle, when non-null.
    pub arrowhead_handle: Option<u64>,
}

impl MLeaderLine {
    /// The per-line override bit for the path type.
    pub const PATH_TYPE: u32 = 1;
    /// The per-line override bit for the arrowhead size.
    pub const ARROWHEAD_SIZE: u32 = 16;
}

/// A start/end pair cut out of a leader segment (DXF `12`/`13`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MLeaderBreak {
    pub start: Point3,
    pub end: Point3,
}

/// One record of complete per-segment break data (DXF `LEADER_LINE` breaks).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderLineBreak {
    pub segment_index: i32,
    pub points: Vec<MLeaderBreak>,
}

/// Source data of one root that this build does not model (`reserved`).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderRootExtras {
    pub root_index: usize,
    /// DXF `290` inside `LEADER{}`.
    pub content_valid: bool,
    /// DXF `291` inside `LEADER{}`.
    pub unknown: bool,
    /// DXF `271` inside `LEADER{}`.
    pub text_attachment_direction: i16,
    /// Root break-point pairs (`12`/`13`).
    pub breaks: Vec<MLeaderBreak>,
}

/// Source data of one leader line that this build does not model (`reserved`).
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderLineExtras {
    pub root_index: usize,
    pub line_index: usize,
    /// DXF line `91`-adjacent segment index.
    pub segment_index: i32,
    /// Complete per-segment break data; the legacy mirrors are derived from it
    /// and are not stored twice.
    pub breaks: Vec<MLeaderLineBreak>,
    /// The legacy break count (DXF `break_info_count`).
    pub break_info_count: i32,
    /// The legacy first-segment break pairs, stored only when `breaks` is empty
    /// so the same source pair is never written twice.
    pub legacy_breaks: Vec<MLeaderBreak>,
    /// Line colour (DXF `92`), preserved as a raw CCR/CMC pair.
    pub line_color: MLeaderColor,
    /// Line type handle (DXF `340`), when non-null.
    pub line_type_handle: Option<u64>,
    /// Line weight (DXF `171`).
    pub line_weight: i16,
}

/// A raw DXF colour: an ACI/CCR index and/or a true-colour value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MLeaderColor {
    /// The ACI index (0 by block, 256 by layer, 257 none, 1..=255 index).
    pub index: Option<u16>,
    /// The packed 24-bit true colour (`(r << 16) | (g << 8) | b`).
    pub true_color: Option<i32>,
}

/// Background-fill parameters (context groups `90`/`91`/`141`/`92`/`291`/`292`).
///
/// Preserved but **not rendered**: the display uses one colour per entity and
/// this build does not draw a background mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MLeaderBackground {
    pub fill_enabled: bool,
    pub mask_fill_on: bool,
    pub fill_color: MLeaderColor,
    pub scale_factor: f64,
    pub transparency: i32,
}

/// Text-column parameters (context groups `173`/`142`/`143`/`144`/`294`).
///
/// Preserved but **not rendered**: this build has no column layout.
#[derive(Debug, Clone, PartialEq)]
pub struct MLeaderColumns {
    pub column_type: i16,
    pub width: f64,
    pub gutter: f64,
    pub flow_reversed: bool,
    pub sizes: Vec<f64>,
}

/// The opaque verbatim source bag (ADR 0019 C.2).
///
/// Partition rule: only source data this build does not model is stored here,
/// always verbatim (never synthesised), never a field that already has a
/// modelled home, and each source group appears exactly once. The importer adds
/// an entry only when the source carried it, so an empty bag means "the file had
/// none of these".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MLeaderReserved {
    /// Unmodelled root data, one entry per source root.
    pub roots: Vec<MLeaderRootExtras>,
    /// Unmodelled per-line data, one entry per source line.
    pub lines: Vec<MLeaderLineExtras>,
    /// Background fill, when the source enabled or set any of it.
    pub background: Option<MLeaderBackground>,
    /// Column layout, when the source set any of it.
    pub columns: Option<MLeaderColumns>,
}

// ---------------------------------------------------------------------------
// The entity.
// ---------------------------------------------------------------------------

/// A first-class MULTILEADER entity (ADR 0019 decision C).
///
/// All fields are private; the only writers are [`Self::new`],
/// [`Self::from_import`] and [`Self::transformed`]. `display` is derived from
/// the other fields, exactly like `HatchGeometry`'s invariant.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderGeometry {
    style: MLeaderStyleValues,
    style_name: String,
    override_mask: u32,
    content: MLeaderContent,
    text: MLeaderText,
    roots: Vec<MLeaderRoot>,
    lines: Vec<MLeaderLine>,
    reserved: MLeaderReserved,
    plane: PlaneFrame,
    display: Vec<SemanticGeometry>,
}

/// The parts accepted by [`MultiLeaderGeometry::from_import`].
///
/// A named struct keeps the import call readable without a twelve-argument
/// constructor (the clippy `too_many_arguments` limit).
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderParts {
    pub style: MLeaderStyleValues,
    pub style_name: String,
    pub override_mask: u32,
    pub content: MLeaderContent,
    pub text: MLeaderText,
    pub roots: Vec<MLeaderRoot>,
    pub lines: Vec<MLeaderLine>,
    pub reserved: MLeaderReserved,
    pub plane: PlaneFrame,
}

impl MultiLeaderGeometry {
    /// Build a multileader on the creation path (P7c), rejecting a display that
    /// cannot be derived.
    ///
    /// The import path uses [`Self::from_import`] instead, which keeps today's
    /// honest degradation instead of rejecting.
    pub fn new(parts: MultiLeaderParts) -> CadResult<Self> {
        let (geometry, reasons) = Self::from_import(parts);
        if let Some(reason) = reasons.first() {
            return Err(CadError::InvalidInput(format!(
                "cannot build the multileader display: {reason}"
            )));
        }
        Ok(geometry)
    }

    /// Build a multileader from imported data, degrading honestly.
    ///
    /// The returned reasons are the display degradations (unapplied colour /
    /// background / columns / breaks, unexpanded custom arrowheads). The
    /// importer folds them into its `Completeness::Partial`.
    pub fn from_import(parts: MultiLeaderParts) -> (Self, Vec<String>) {
        let mut geometry = MultiLeaderGeometry {
            style: parts.style,
            style_name: parts.style_name,
            override_mask: parts.override_mask,
            content: parts.content,
            text: parts.text,
            roots: parts.roots,
            lines: parts.lines,
            reserved: parts.reserved,
            plane: parts.plane,
            display: Vec::new(),
        };
        let (display, reasons) = geometry.build_display();
        geometry.display = display;
        (geometry, reasons)
    }

    /// Transform by a **rigid** motion (translation + rotation) and rebuild the
    /// display (ADR 0019 C.5).
    ///
    /// Scaling (even uniform), mirroring and non-uniform scaling are refused:
    /// they would change the text height/arrowhead size/dogleg length while the
    /// `90` override mask stays unchanged, so a reload that re-resolves masked
    /// values from the style would disagree with what we display. The caller's
    /// selection transform is all-or-nothing, so the whole command fails.
    pub fn transformed(&self, transform: &Transform3) -> CadResult<Self> {
        if !is_rigid(transform) {
            return Err(CadError::Unsupported(
                "multileader transform requires a translation + rotation; scaling (uniform or \
                 not) and mirroring are refused because they would change text height, arrowhead \
                 size and dogleg length while the override mask keeps its bits"
                    .to_string(),
            ));
        }
        let p = |point: Point3| transform.apply_point(point);
        let v = |vector: Point3| transform.apply_vector(vector);
        let delta = rotation_delta(transform);
        let mut roots = self.roots.clone();
        for root in &mut roots {
            root.landing_point = p(root.landing_point);
            root.dogleg_vector = v(root.dogleg_vector);
        }
        let mut lines = self.lines.clone();
        for line in &mut lines {
            line.points = line.points.iter().map(|point| p(*point)).collect();
        }
        let mut reserved = self.reserved.clone();
        for root in &mut reserved.roots {
            for point in &mut root.breaks {
                point.start = p(point.start);
                point.end = p(point.end);
            }
        }
        for line in &mut reserved.lines {
            for record in &mut line.breaks {
                for point in &mut record.points {
                    point.start = p(point.start);
                    point.end = p(point.end);
                }
            }
            for point in &mut line.legacy_breaks {
                point.start = p(point.start);
                point.end = p(point.end);
            }
        }
        let mut text = self.text.clone();
        text.location = p(text.location);
        // The text rotation is the display's in-plane angle; a rigid motion adds
        // the same XY rotation the other display kinds accumulate.
        text.rotation += delta;
        let plane = PlaneFrame {
            origin: p(self.plane.origin),
            u: v(self.plane.u),
            v: v(self.plane.v),
            normal_reversed: self.plane.normal_reversed,
        };
        let mut geometry = MultiLeaderGeometry {
            style: self.style.clone(),
            style_name: self.style_name.clone(),
            override_mask: self.override_mask,
            content: self.content,
            text,
            roots,
            lines,
            reserved,
            plane,
            display: Vec::new(),
        };
        // Degradations that already existed before the transform are not new and
        // must not make the transform fail; rebuild the geometry exactly like
        // the constructor's and discard the repeated reasons.
        geometry.display = geometry.build_display().0;
        Ok(geometry)
    }

    pub fn style(&self) -> &MLeaderStyleValues {
        &self.style
    }

    pub fn style_name(&self) -> &str {
        &self.style_name
    }

    /// The raw DXF `90` override mask, preserved bit for bit.
    pub fn override_mask(&self) -> u32 {
        self.override_mask
    }

    pub fn content(&self) -> MLeaderContent {
        self.content
    }

    pub fn text(&self) -> &MLeaderText {
        &self.text
    }

    pub fn roots(&self) -> &[MLeaderRoot] {
        &self.roots
    }

    pub fn lines(&self) -> &[MLeaderLine] {
        &self.lines
    }

    pub fn reserved(&self) -> &MLeaderReserved {
        &self.reserved
    }

    pub fn plane(&self) -> PlaneFrame {
        self.plane
    }

    /// The derived display: leader paths, arrowheads, doglegs and text.
    pub fn display(&self) -> &[SemanticGeometry] {
        &self.display
    }

    /// The lines belonging to one root, in source order.
    pub fn root_lines(&self, root_index: usize) -> impl Iterator<Item = &MLeaderLine> {
        self.lines
            .iter()
            .filter(move |line| line.root_index == root_index)
    }

    /// Derive the display from `style + text + roots + lines`.
    ///
    /// This is a verbatim move of the pre-P7b importer synthesis; the order is
    /// root by root (path, arrowhead, dogleg), then the annotation text. Returns
    /// the display and the honest reasons for preserved-but-unrendered data.
    fn build_display(&self) -> (Vec<SemanticGeometry>, Vec<String>) {
        let mut children: Vec<SemanticGeometry> = Vec::new();
        let mut custom_arrowhead = false;
        for (root_index, root) in self.roots.iter().enumerate() {
            for line in self.root_lines(root_index) {
                if line.points.len() < 2 {
                    continue;
                }
                let path_type = if line.override_flags & MLeaderLine::PATH_TYPE != 0 {
                    line.path_type
                } else {
                    self.style.path_type
                };
                if let Some(path) = path_geometry(&line.points, path_type) {
                    children.push(path);
                }
                if path_type == MLeaderPathType::Invisible {
                    continue;
                }
                if line.arrowhead_handle.is_some_and(|handle| handle != 0) {
                    custom_arrowhead = true;
                }
                let arrow_size = if line.override_flags & MLeaderLine::ARROWHEAD_SIZE != 0
                    && line.arrowhead_size.is_finite()
                    && line.arrowhead_size > 0.0
                {
                    line.arrowhead_size
                } else {
                    self.style.arrowhead_size
                };
                let outward = sub(line.points[0], line.points[1]);
                if arrow_size.is_finite() && arrow_size > 1e-9 && length(outward) > 1e-9 {
                    children.push(arrowhead(line.points[0], outward, arrow_size));
                }
            }
            if self.style.enable_dogleg {
                if let Some(dogleg) = self.dogleg(root_index, root) {
                    children.push(dogleg);
                }
            }
        }
        if self.content == MLeaderContent::Text && !self.text.string.is_empty() {
            children.push(self.text_geometry());
        }

        let mut reasons: Vec<String> = vec![
            "multileader colour, lineweight, linetype and property overrides are resolved but not \
             applied (the representation carries one colour per entity)"
                .into(),
        ];
        if let Some(background) = &self.reserved.background {
            if background.fill_enabled {
                reasons.push("multileader background fill is not applied".into());
            }
        }
        if self.reserved.columns.is_some() {
            reasons.push("multileader column layout is not applied".into());
        }
        if self
            .reserved
            .roots
            .iter()
            .any(|root| !root.breaks.is_empty())
            || self
                .reserved
                .lines
                .iter()
                .any(|line| !line.breaks.is_empty() || !line.legacy_breaks.is_empty())
        {
            reasons.push("multileader leader-line breaks are preserved but not rendered".into());
        }
        if custom_arrowhead {
            reasons.push("custom multileader arrowhead blocks are not expanded".into());
        }
        if !plane_is_identity(&self.plane) {
            reasons.push("multileader plane frame is preserved but not applied".into());
        }
        (children, reasons)
    }

    /// The annotation text as a `Text` display child (the pre-P7b placement).
    fn text_geometry(&self) -> SemanticGeometry {
        SemanticGeometry::Text {
            text: self.text.string.clone(),
            position: self.text.location,
            style: self.style.text_style,
            height: self.text.height,
            rotation: self.text.rotation,
            font: self.style.text_font.clone(),
            h_align: self.text.h_align(),
            v_align: self.text.v_align(),
        }
    }

    /// The dogleg / landing segment at the content end of a leader root.
    ///
    /// Verbatim from the pre-P7b importer: it starts at the **last vertex of the
    /// first line that has one**, runs toward the connection point for
    /// `landing_distance` (falling back to the style/entity default), stopping
    /// `landing_gap` short. `reach < 1e-9` draws nothing.
    fn dogleg(&self, root_index: usize, root: &MLeaderRoot) -> Option<SemanticGeometry> {
        let start = self
            .root_lines(root_index)
            .find_map(|line| line.points.last().copied())?;
        if !is_finite_point(start) {
            return None;
        }
        let length = if root.dogleg_length.is_finite() && root.dogleg_length > 1e-9 {
            root.dogleg_length
        } else if self.style.landing_distance.is_finite() {
            self.style.landing_distance
        } else {
            0.0
        };
        let reach = (length - self.style.landing_gap.max(0.0)).max(0.0);
        if reach < 1e-9 {
            return None;
        }
        let connection = root.landing_point;
        let toward_content = sub(connection, start);
        let direction = if is_finite_point(connection) && length3(toward_content) > 1e-9 {
            normalize(toward_content)
        } else {
            let fallback = root.dogleg_vector;
            if is_finite_point(fallback) && length3(fallback) > 1e-9 {
                normalize(fallback)
            } else {
                Point3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                }
            }
        };
        let end = add(start, scale(direction, reach));
        line_between(start, end)
    }
}

/// Build one leader path honoring its `path_type`.
///
/// `Invisible` draws nothing, `StraightLineSegments` a polyline, and `Spline`
/// the same B-spline through the same points the pre-P7b importer emitted.
fn path_geometry(points: &[Point3], path_type: MLeaderPathType) -> Option<SemanticGeometry> {
    match path_type {
        MLeaderPathType::Invisible => None,
        MLeaderPathType::StraightLineSegments => Some(SemanticGeometry::Polyline {
            points: points.to_vec(),
            bulges: Vec::new(),
            closed: false,
        }),
        MLeaderPathType::Spline => {
            let degree = 3.min(points.len().saturating_sub(1).max(1)) as u32;
            Some(SemanticGeometry::Spline {
                degree,
                knots: Vec::new(),
                control_points: points.to_vec(),
                weights: Vec::new(),
            })
        }
    }
}

fn length3(v: Point3) -> f64 {
    v.x.hypot(v.y).hypot(v.z)
}

/// Whether the plane frame is the default `+Z` frame (a non-default frame is
/// preserved but never applied by the display).
fn plane_is_identity(plane: &PlaneFrame) -> bool {
    let default = PlaneFrame::default();
    plane.normal_reversed == default.normal_reversed
        && plane.origin == default.origin
        && plane.u == default.u
        && plane.v == default.v
}

/// Whether a transform is a translation + rotation (no scale, no mirror).
fn is_rigid(transform: &Transform3) -> bool {
    let m = &transform.matrix;
    // Reject a perspective row or a non-affine homogeneous coordinate.
    if m[3][0].abs() > 1e-9
        || m[3][1].abs() > 1e-9
        || m[3][2].abs() > 1e-9
        || (m[3][3] - 1.0).abs() > 1e-9
    {
        return false;
    }
    let column = |index: usize| Point3 {
        x: m[0][index],
        y: m[1][index],
        z: m[2][index],
    };
    let c0 = column(0);
    let c1 = column(1);
    let c2 = column(2);
    let unit = |v: Point3| (length3(v) - 1.0).abs() <= 1e-9;
    let orthogonal = |a: Point3, b: Point3| (a.x * b.x + a.y * b.y + a.z * b.z).abs() <= 1e-9;
    unit(c0)
        && unit(c1)
        && unit(c2)
        && orthogonal(c0, c1)
        && orthogonal(c1, c2)
        && orthogonal(c0, c2)
        && transform.determinant() > 0.0
}

/// The rotation a rigid transform applies to the XY plane, in radians.
fn rotation_delta(transform: &Transform3) -> f64 {
    let v = transform.apply_vector(Point3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    });
    v.y.atan2(v.x)
}
