//! First-class HATCH entity: boundary loops in hatch-plane 2D, the pattern /
//! gradient definition, and the derived display geometry (ADR 0018 A.1–A.3).
//!
//! [`HatchGeometry`] is the single builder of a hatch's display: it is derived
//! from `loops + style + fill + plane` by [`HatchGeometry::build_display`], and
//! the only writers are [`HatchGeometry::new`] (creation path),
//! [`HatchGeometry::from_import`] (import path, which never rejects) and
//! [`HatchGeometry::transformed`]. This mirrors `DimensionGeometry`'s invariant
//! so a caller cannot store a display that disagrees with the definition.
//!
//! The fill geometry it consumes lives in [`crate::hatch`] and is reused
//! unchanged; this module only owns the entity semantics (loop set filtering by
//! style, the pattern-definition composite and the world placement).

use crate::hatch::{
    fill_rings, gradient_vertex_colors, pattern_polylines, simplify, FillError, GradientDef,
    GradientKind, GradientStop, PatternLine,
};
use crate::{CadError, CadResult, Mesh, Point3, SemanticGeometry, Transform3};

use geometry::{add3, dot3, length3, scale3, sub3};
use geometry::{
    arbitrary_axis, column_scale, fill_tolerance, finite_point, normalize3, sample_loop,
};
use validate::{loop_depths, plane_is_usable, validate_creation};

mod geometry;
mod validate;

// ---------------------------------------------------------------------------
// Boundary loops and their source edges (ADR 0018 A.1/A.2, B4).
// ---------------------------------------------------------------------------

/// Raw boundary-path flag bits (DXF group 92).
///
/// The importer stores the source bits verbatim: five booleans would silently
/// normalise away the bits this build does not interpret (`TEXTBOX` and the
/// poorly defined `92=1/16/0` distinction; ADR N3/U-5). Only the load-bearing
/// bits get accessors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HatchLoopFlags(pub u32);

impl HatchLoopFlags {
    pub const EXTERNAL: u32 = 1;
    pub const POLYLINE: u32 = 2;
    /// A derived boundary (the file did not store its vertices).
    pub const DERIVED: u32 = 4;
    pub const TEXTBOX: u32 = 8;

    /// The raw bits (DXF group 92).
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Whether the loop is a derived boundary; only such a hatch writes group
    /// 47 (ADR B3).
    pub fn derived(self) -> bool {
        self.0 & Self::DERIVED != 0
    }

    /// Whether the loop is a text box. Preserved but **not interpreted**
    /// (ADR U-5).
    pub fn textbox(self) -> bool {
        self.0 & Self::TEXTBOX != 0
    }
}

/// One analytic boundary edge of the source path (ADR 0018 A.1/B4).
///
/// Mirrors acadrust's edge kinds; `Polyline` paths are not analytic and are
/// represented by the loop's points/bulges instead, so this enum has no
/// polyline variant. The spline variant keeps the source control points with
/// the weight in `[2]`, exactly as the DXF record stores them.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryEdge {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    CircularArc {
        center: [f64; 2],
        radius: f64,
        start_angle: f64,
        end_angle: f64,
        ccw: bool,
    },
    EllipticArc {
        center: [f64; 2],
        /// Endpoint of the major axis relative to the centre.
        major_axis: [f64; 2],
        ratio: f64,
        start_angle: f64,
        end_angle: f64,
        ccw: bool,
    },
    Spline {
        degree: u32,
        rational: bool,
        periodic: bool,
        knots: Vec<f64>,
        /// Control points; the weight of a rational spline rides in `[2]`.
        control_points: Vec<[f64; 3]>,
        fit_points: Vec<[f64; 2]>,
        start_tangent: [f64; 2],
        end_tangent: [f64; 2],
    },
}

/// One closed boundary loop in **hatch-plane 2D** coordinates (OCS).
///
/// `points` is implicitly closed (no repeated last point) and `bulges` follows
/// the DXF convention (`tan(included_angle / 4)`); an empty `bulges` means every
/// segment is straight. `source_edges` keeps the parsed analytic edges when the
/// source carried them, so an export can write them back; a `Polyline` path (or
/// a loop built from scratch) has none and degrades to the sampled points
/// (ADR B4, declared).
#[derive(Debug, Clone, PartialEq)]
pub struct HatchLoop {
    pub points: Vec<[f64; 2]>,
    pub bulges: Vec<f64>,
    pub flags: HatchLoopFlags,
    pub source_edges: Option<Vec<BoundaryEdge>>,
}

impl HatchLoop {
    /// A loop with an explicit flag word and no analytic source edges.
    pub fn new(points: Vec<[f64; 2]>, bulges: Vec<f64>, flags: HatchLoopFlags) -> Self {
        HatchLoop {
            points,
            bulges,
            flags,
            source_edges: None,
        }
    }
}

/// The hatch plane: the OCS normal (DXF group 210) and its elevation along that
/// normal (groups 10/20 are always 0, group 30 is the elevation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HatchPlane {
    pub normal: Point3,
    pub elevation: f64,
}

impl Default for HatchPlane {
    fn default() -> Self {
        HatchPlane {
            normal: Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            elevation: 0.0,
        }
    }
}

impl HatchPlane {
    pub fn new(normal: Point3, elevation: f64) -> Self {
        HatchPlane { normal, elevation }
    }

    /// The hatch-plane origin in world space: `normal * elevation`.
    ///
    /// The unit normal comes from [`arbitrary_axis`] (which falls back to +Z for
    /// a degenerate normal), so a degenerate plane keeps the same boundary
    /// placement the pre-P6a importer produced.
    fn origin(&self) -> Point3 {
        let (_, _, un) = arbitrary_axis(self.normal);
        scale3(un, self.elevation)
    }

    /// Map a hatch-plane 2D point into world space.
    pub fn to_world(&self, p: [f64; 2]) -> Point3 {
        let (ux, uy, _) = arbitrary_axis(self.normal);
        add3(self.origin(), add3(scale3(ux, p[0]), scale3(uy, p[1])))
    }

    /// The world point at the plane origin.
    pub fn world_origin(&self) -> Point3 {
        self.origin()
    }
}

// ---------------------------------------------------------------------------
// Style, pattern and fill (ADR 0018 A.1/A.3).
// ---------------------------------------------------------------------------

/// HATCH style (DXF group 75). Style is a **loop-set filter**, not a separate
/// fill rule: the display builder uses the accepted loops and the unchanged
/// even-odd fill (ADR A.3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HatchStyle {
    /// Odd-parity area: every accepted loop (DXF "normal").
    #[default]
    Normal,
    /// Outermost area only: drop loops at depth >= 3 (1-based).
    Outer,
    /// Through the entire area: drop loops at depth >= 2 (1-based).
    Ignore,
}

/// HATCH pattern type (DXF group 76).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HatchPatternType {
    /// A user-defined pattern (`76=0`). A file may omit the definition rows and
    /// carry only angle/spacing/double.
    #[default]
    UserDefined,
    /// A predefined pattern (`76=1`).
    Predefined,
    /// A custom pattern (`76=2`).
    Custom,
}

/// One pattern-definition line, in **pattern space** (DXF groups 53/43/44/45/46
/// and 49). The hatch-level angle (group 52) and scale (group 41) are applied by
/// [`HatchPattern::families`], so these values round-trip unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    pub angle: f64,
    pub base_x: f64,
    pub base_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub dashes: Vec<f64>,
}

/// The HATCH pattern definition: name, hatch-level angle/scale, the double flag
/// and the definition rows (ADR A.1).
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPattern {
    pub name: String,
    /// DXF group 52: pattern angle in radians.
    pub angle: f64,
    /// DXF group 41: pattern scale. For a user-defined pattern this is the
    /// spacing between the synthesized line families.
    pub scale: f64,
    /// DXF group 77: draw a second family perpendicular to the first.
    pub double: bool,
    pub lines: Vec<HatchPatternLine>,
}

impl HatchPattern {
    /// Compose the definition rows into hatch-plane line families.
    ///
    /// The composite is **our implementation** (ADR U-1): the hatch-level angle
    /// rotates both the base point and the offset, and the hatch-level scale
    /// scales the base point, the offset and the dash lengths. Before P6a the
    /// importer rotated only the base point, did not scale it and did not rotate
    /// the offset (ADR U-7); both are fixed here and pinned by a numeric test.
    /// The `double` flag adds a family at +90 degrees whose offset is the first
    /// family's offset rotated by +90 degrees.
    pub fn families(&self) -> Vec<PatternLine> {
        let (sin, cos) = self.angle.sin_cos();
        let rotate = |x: f64, y: f64| [cos * x - sin * y, sin * x + cos * y];
        let mut out = Vec::new();
        for line in &self.lines {
            let base = rotate(self.scale * line.base_x, self.scale * line.base_y);
            let offset = rotate(self.scale * line.offset_x, self.scale * line.offset_y);
            let dashes: Vec<f64> = line.dashes.iter().map(|d| d * self.scale).collect();
            let angle = line.angle + self.angle;
            out.push(PatternLine {
                angle,
                base,
                offset,
                dashes: dashes.clone(),
            });
            if self.double {
                out.push(PatternLine {
                    angle: angle + std::f64::consts::FRAC_PI_2,
                    base,
                    offset: [-offset[1], offset[0]],
                    dashes,
                });
            }
        }
        out
    }

    /// Add a family rotation (in-plane radians) and scale, keeping the raw
    /// definition rows untouched.
    fn transformed(&self, delta: f64, scale: f64) -> Self {
        HatchPattern {
            name: self.name.clone(),
            angle: self.angle + delta,
            scale: self.scale * scale,
            double: self.double,
            lines: self.lines.clone(),
        }
    }
}

/// A gradient fill (ADR A.1/B3).
///
/// The variant is the DXF colour mode; `name` carries the gradient kind
/// (`LINEAR`, `SPHERICAL`, ...). Group 460 (`angle_raw`) is kept **opaque**: the
/// writer and reader pass it through unchanged while the structure comment
/// claims radians, so its unit is unverified (ADR U-13) and it is never
/// converted here.
#[derive(Debug, Clone, PartialEq)]
pub enum HatchGradient {
    TwoColor {
        name: String,
        reserved: i32,
        angle_raw: f64,
        shift: f64,
        stops: Vec<GradientStop>,
    },
    SingleColor {
        name: String,
        reserved: i32,
        angle_raw: f64,
        shift: f64,
        tint: f64,
        stops: Vec<GradientStop>,
    },
}

impl HatchGradient {
    pub fn name(&self) -> &str {
        match self {
            HatchGradient::TwoColor { name, .. } | HatchGradient::SingleColor { name, .. } => name,
        }
    }

    fn stops(&self) -> &[GradientStop] {
        match self {
            HatchGradient::TwoColor { stops, .. } | HatchGradient::SingleColor { stops, .. } => {
                stops
            }
        }
    }

    /// The gradient kind this build can bake, or `None` for a curved/inversion
    /// kind (declared unsupported, never approximated as a straight ramp).
    fn kind(&self) -> Option<GradientKind> {
        match self.name().trim().to_ascii_uppercase().as_str() {
            "LINEAR" => Some(GradientKind::Linear),
            "SPHERICAL" | "CYLINDER" => Some(GradientKind::Spherical),
            // A missing name is ambiguous; fall back to LINEAR only when the
            // file actually carried stops (the pre-P6a rule, preserved).
            "" if !self.stops().is_empty() => Some(GradientKind::Linear),
            _ => None,
        }
    }

    /// The resolved bake definition, or `None` when the gradient is outside the
    /// implemented subset (unsupported kind or unusable stops).
    fn definition(&self) -> Option<GradientDef> {
        let kind = self.kind()?;
        let (shift, single_color, tint) = match self {
            HatchGradient::TwoColor { shift, .. } => (*shift, false, 0.0),
            HatchGradient::SingleColor { shift, tint, .. } => (*shift, true, *tint),
        };
        let def = GradientDef {
            kind,
            angle: match self {
                HatchGradient::TwoColor { angle_raw, .. }
                | HatchGradient::SingleColor { angle_raw, .. } => *angle_raw,
            },
            shift,
            single_color,
            tint,
            stops: self.stops().to_vec(),
        };
        def.is_usable().then_some(def)
    }
}

/// The pattern / gradient definition of a hatch.
#[derive(Debug, Clone, PartialEq)]
pub enum HatchFill {
    Solid,
    Pattern(HatchPattern),
    Gradient(HatchGradient),
}

// ---------------------------------------------------------------------------
// The entity (ADR 0018 A.1).
// ---------------------------------------------------------------------------

/// A first-class HATCH entity.
///
/// All fields are private; the only writers are [`HatchGeometry::new`] (creation
/// path, which rejects the ADR A.3 boundary defects), [`HatchGeometry::from_import`]
/// (import path, which degrades honestly and never rejects) and
/// [`HatchGeometry::transformed`]. `display` is derived from the other fields
/// (ADR A.4 invariant).
#[derive(Debug, Clone, PartialEq)]
pub struct HatchGeometry {
    loops: Vec<HatchLoop>,
    plane: HatchPlane,
    style: HatchStyle,
    fill: HatchFill,
    seeds: Vec<[f64; 2]>,
    pixel_size: Option<f64>,
    pattern_type: HatchPatternType,
    display: Vec<SemanticGeometry>,
}

impl HatchGeometry {
    /// Build a hatch on the creation path (ADR A.3).
    ///
    /// Rejects a boundary this build cannot represent exactly: a degenerate
    /// plane, fewer than three finite vertices, a self-intersecting loop, rings
    /// that cross or touch, duplicate rings, and a fill that cannot be
    /// generated. The import path uses [`Self::from_import`] instead, which
    /// keeps today's honest degradation (ADR B6: import does not adopt the
    /// creation-path rejections).
    pub fn new(
        loops: Vec<HatchLoop>,
        plane: HatchPlane,
        style: HatchStyle,
        fill: HatchFill,
        seeds: Vec<[f64; 2]>,
        pixel_size: Option<f64>,
        pattern_type: HatchPatternType,
    ) -> CadResult<Self> {
        validate_creation(&loops, &plane)?;
        let mut hatch = HatchGeometry {
            loops,
            plane,
            style,
            fill,
            seeds,
            pixel_size,
            pattern_type,
            display: Vec::new(),
        };
        let (display, reasons) = hatch.build_display();
        if let Some(reason) = reasons.into_iter().next() {
            return Err(CadError::InvalidInput(format!(
                "cannot build the hatch display: {reason}"
            )));
        }
        hatch.display = display;
        Ok(hatch)
    }

    /// Build a hatch from imported data, degrading honestly (ADR B6).
    ///
    /// The returned reasons are the display degradations (boundary-only fills,
    /// unsupported gradients); the importer folds them into its `Completeness`.
    /// Nothing is ever rejected: a loop set that cannot be filled still shows
    /// its boundaries.
    #[allow(clippy::too_many_arguments)]
    pub fn from_import(
        loops: Vec<HatchLoop>,
        plane: HatchPlane,
        style: HatchStyle,
        fill: HatchFill,
        seeds: Vec<[f64; 2]>,
        pixel_size: Option<f64>,
        pattern_type: HatchPatternType,
    ) -> (Self, Vec<String>) {
        let mut hatch = HatchGeometry {
            loops,
            plane,
            style,
            fill,
            seeds,
            pixel_size,
            pattern_type,
            display: Vec::new(),
        };
        let (display, reasons) = if plane_is_usable(&hatch.plane) {
            hatch.build_display()
        } else {
            (
                hatch.boundary_display(),
                vec!["hatch plane normal is degenerate; boundary only".to_string()],
            )
        };
        hatch.display = display;
        (hatch, reasons)
    }

    /// Transform the loops, plane, seeds and pattern definition by a similarity,
    /// then rebuild `display` (ADR E).
    ///
    /// A non-uniform scale or a mirror is refused for the whole entity: a hatch
    /// plane cannot be represented under a non-similarity without inventing an
    /// approximation.
    pub fn transformed(&self, transform: &Transform3) -> CadResult<Self> {
        if !transform.is_uniform_scale(1e-9) || transform.determinant() <= 0.0 {
            return Err(CadError::Unsupported(
                "hatch transform requires a rotation and uniform scale; mirror or non-uniform \
                 scale cannot be represented"
                    .to_string(),
            ));
        }
        let scale = column_scale(transform);
        let mapped_normal = normalize3(transform.apply_vector(self.plane.normal));
        if !finite_point(mapped_normal) || length3(mapped_normal) < 1e-12 {
            return Err(CadError::Unsupported(
                "hatch transform collapses the hatch plane normal".to_string(),
            ));
        }
        // The new OCS origin is the transformed old origin projected onto the
        // new normal (the OCS origin is constrained to `normal * elevation`).
        let new_origin = transform.apply_point(self.plane.origin());
        let elevation = dot3(new_origin, mapped_normal);
        let new_plane = HatchPlane {
            normal: mapped_normal,
            elevation,
        };
        let (ux, uy, _) = arbitrary_axis(mapped_normal);
        let origin = new_plane.origin();
        let to_local = |p: Point3| {
            let rel = sub3(p, origin);
            [dot3(rel, ux), dot3(rel, uy)]
        };
        let map_point = |p: [f64; 2]| to_local(transform.apply_point(self.plane.to_world(p)));

        // The in-plane rotation the similarity induces in the new basis. The
        // pattern definition lives in this frame, so its angle must follow it
        // (the ADR leaves this open; without it a rotated hatch keeps its
        // pre-rotation pattern direction and disagrees with its own display).
        let local_origin = map_point([0.0, 0.0]);
        let local_x = map_point([1.0, 0.0]);
        let delta = (local_x[1] - local_origin[1]).atan2(local_x[0] - local_origin[0]);

        let loops = self
            .loops
            .iter()
            .map(|loop_| HatchLoop {
                points: loop_.points.iter().map(|p| map_point(*p)).collect(),
                bulges: loop_.bulges.clone(),
                flags: loop_.flags,
                // Source edges live in the same OCS frame as the points, so
                // they follow the same similarity; keeping them verbatim
                // would export stale analytic edges.
                source_edges: loop_
                    .source_edges
                    .as_ref()
                    .map(|edges| transform_edges(edges, &map_point, delta, scale)),
            })
            .collect();
        let seeds = self.seeds.iter().map(|p| map_point(*p)).collect();
        let fill = match &self.fill {
            HatchFill::Solid => HatchFill::Solid,
            HatchFill::Pattern(pattern) => HatchFill::Pattern(pattern.transformed(delta, scale)),
            // `angle_raw` is opaque (ADR U-13): converting it would invent a
            // unit; the data is preserved verbatim and the bake direction is
            // declared unverified.
            HatchFill::Gradient(gradient) => HatchFill::Gradient(gradient.clone()),
        };
        let mut hatch = HatchGeometry {
            loops,
            plane: new_plane,
            style: self.style,
            fill,
            seeds,
            pixel_size: self.pixel_size,
            pattern_type: self.pattern_type,
            display: Vec::new(),
        };
        // Degradations already present before the transform are not new and must
        // not make a transform fail; the display is rebuilt exactly like the
        // constructor's (the reasons are discarded, the geometry is not).
        hatch.display = hatch.build_display().0;
        Ok(hatch)
    }

    pub fn loops(&self) -> &[HatchLoop] {
        &self.loops
    }

    pub fn plane(&self) -> HatchPlane {
        self.plane
    }

    pub fn style(&self) -> HatchStyle {
        self.style
    }

    pub fn fill(&self) -> &HatchFill {
        &self.fill
    }

    pub fn seeds(&self) -> &[[f64; 2]] {
        &self.seeds
    }

    pub fn pixel_size(&self) -> Option<f64> {
        self.pixel_size
    }

    pub fn pattern_type(&self) -> HatchPatternType {
        self.pattern_type
    }

    /// The derived display: every boundary loop plus the style-filtered fill or
    /// pattern geometry.
    pub fn display(&self) -> &[SemanticGeometry] {
        &self.display
    }

    /// Precise local bounds from the boundary vertices, or `None` when a bulge
    /// makes the chord bound non-conservative.
    pub fn local_bounds(&self) -> Option<(Point3, Point3)> {
        if self
            .loops
            .iter()
            .any(|loop_| loop_.bulges.iter().any(|b| b.abs() > 1e-12))
        {
            return None;
        }
        let mut min = Point3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: f64::INFINITY,
        };
        let mut max = Point3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        };
        let mut any = false;
        for loop_ in &self.loops {
            for p in &loop_.points {
                let world = self.plane.to_world(*p);
                if !finite_point(world) {
                    return None;
                }
                any = true;
                min.x = min.x.min(world.x);
                min.y = min.y.min(world.y);
                min.z = min.z.min(world.z);
                max.x = max.x.max(world.x);
                max.y = max.y.max(world.y);
                max.z = max.z.max(world.z);
            }
        }
        any.then_some((min, max))
    }

    /// Boundary-loop polylines only (the plane is unusable, so no fill).
    fn boundary_display(&self) -> Vec<SemanticGeometry> {
        self.loops
            .iter()
            .map(|loop_| SemanticGeometry::Polyline {
                points: loop_
                    .points
                    .iter()
                    .map(|p| self.plane.to_world(*p))
                    .collect(),
                bulges: loop_.bulges.clone(),
                closed: true,
            })
            .collect()
    }

    /// Derive the display from `loops + style + fill + plane` (ADR A.3/A.4).
    ///
    /// Boundary polylines are **always** present (all loops, so every boundary
    /// vertex stays a snap point and the pick parity matches a compound); style
    /// only filters the loop set fed to the fill / pattern / gradient. Returns
    /// the display and the honest reasons for any fill that could not be
    /// generated.
    fn build_display(&self) -> (Vec<SemanticGeometry>, Vec<String>) {
        let mut display = self.boundary_display();
        let mut reasons = Vec::new();
        let filtered = self.filtered_loops();
        let sampled: Vec<Vec<[f64; 2]>> = filtered.iter().map(|loop_| sample_loop(loop_)).collect();
        match &self.fill {
            HatchFill::Solid => match self.fill_mesh(&sampled) {
                Ok(mesh) => display.push(self.mesh_geometry(mesh, Vec::new())),
                Err(error) => reasons.push(format!(
                    "solid hatch could not be filled: {}",
                    error.reason()
                )),
            },
            HatchFill::Pattern(pattern) => {
                let families = pattern.families();
                if families.is_empty() {
                    reasons.push("hatch pattern has no line families".to_string());
                } else {
                    for line in pattern_polylines(&sampled, &families) {
                        if line.len() >= 2 {
                            display.push(SemanticGeometry::Polyline {
                                points: line.iter().map(|p| self.plane.to_world(*p)).collect(),
                                bulges: Vec::new(),
                                closed: false,
                            });
                        }
                    }
                }
            }
            HatchFill::Gradient(gradient) => match gradient.definition() {
                None => {
                    let code = if gradient.kind().is_some() {
                        "gradient_definition_unusable"
                    } else {
                        "gradient_kind_not_supported"
                    };
                    reasons.push(format!(
                        "gradient hatch not rendered ({code}); boundary only"
                    ));
                }
                Some(def) => match self.fill_mesh(&sampled) {
                    Ok(fill) => {
                        let colors = gradient_vertex_colors(&fill, &def);
                        if colors.len() != fill.vertices.len() {
                            reasons.push(
                                "gradient hatch could not be baked; boundary only".to_string(),
                            );
                        } else {
                            display.push(self.mesh_geometry(fill, colors));
                        }
                    }
                    Err(error) => reasons.push(format!(
                        "gradient hatch could not be filled: {}",
                        error.reason()
                    )),
                },
            },
        }
        (display, reasons)
    }

    /// The accepted loops for this style (ADR A.3, 1-based depth).
    fn filtered_loops(&self) -> Vec<&HatchLoop> {
        let depths = loop_depths(&self.loops);
        self.loops
            .iter()
            .zip(depths)
            .filter(|(_, depth)| match self.style {
                HatchStyle::Normal => true,
                // Drop depth >= 3, i.e. two or more enclosing loops.
                HatchStyle::Outer => *depth < 2,
                // Drop depth >= 2, i.e. any enclosing loop.
                HatchStyle::Ignore => *depth == 0,
            })
            .map(|(loop_, _)| loop_)
            .collect()
    }

    /// Even-odd fill of the (already filtered) sampled loops, simplified to keep
    /// the triangulation bounded exactly like the pre-P6a importer.
    fn fill_mesh(&self, sampled: &[Vec<[f64; 2]>]) -> Result<crate::hatch::FillMesh, FillError> {
        let tolerance = fill_tolerance(sampled);
        let simplified: Vec<Vec<[f64; 2]>> = sampled
            .iter()
            .filter_map(|loop_| {
                let mut closed = loop_.clone();
                if closed.is_empty() {
                    return None;
                }
                closed.push(closed[0]);
                let mut simplified = simplify(&closed, tolerance);
                if simplified.len() > 1 && simplified.last() == simplified.first() {
                    simplified.pop();
                }
                (simplified.len() >= 3).then_some(simplified)
            })
            .collect();
        fill_rings(&simplified)
    }

    fn mesh_geometry(
        &self,
        fill: crate::hatch::FillMesh,
        colors: Vec<[u8; 3]>,
    ) -> SemanticGeometry {
        let normal = normalize3(self.plane.normal);
        let vertices: Vec<Point3> = fill
            .vertices
            .iter()
            .map(|p| self.plane.to_world(*p))
            .collect();
        let normals = vec![normal; vertices.len()];
        SemanticGeometry::Mesh(Mesh {
            vertices,
            triangles: fill.triangles,
            normals,
            face_sources: Vec::new(),
            colors,
        })
    }
}

/// Transform analytic source edges into the new hatch-plane frame.
///
/// A similarity keeps a circular arc circular and an elliptic arc elliptic, so
/// this re-expresses the edges exactly: centres/endpoints are mapped, lengths
/// scale and the in-plane angles rotate by `delta`. Spline control points keep
/// their weights (the `z` component) and knot vector; only the 2D location and
/// the tangents move.
fn transform_edges(
    edges: &[BoundaryEdge],
    map_point: &impl Fn([f64; 2]) -> [f64; 2],
    delta: f64,
    scale: f64,
) -> Vec<BoundaryEdge> {
    let rotate = |v: [f64; 2]| {
        let (sin, cos) = delta.sin_cos();
        [cos * v[0] - sin * v[1], sin * v[0] + cos * v[1]]
    };
    edges
        .iter()
        .map(|edge| match edge {
            BoundaryEdge::Line { start, end } => BoundaryEdge::Line {
                start: map_point(*start),
                end: map_point(*end),
            },
            BoundaryEdge::CircularArc {
                center,
                radius,
                start_angle,
                end_angle,
                ccw,
            } => BoundaryEdge::CircularArc {
                center: map_point(*center),
                radius: radius * scale,
                start_angle: start_angle + delta,
                end_angle: end_angle + delta,
                ccw: *ccw,
            },
            BoundaryEdge::EllipticArc {
                center,
                major_axis,
                ratio,
                start_angle,
                end_angle,
                ccw,
            } => {
                let axis = rotate([major_axis[0] * scale, major_axis[1] * scale]);
                BoundaryEdge::EllipticArc {
                    center: map_point(*center),
                    major_axis: axis,
                    ratio: *ratio,
                    start_angle: start_angle + delta,
                    end_angle: end_angle + delta,
                    ccw: *ccw,
                }
            }
            BoundaryEdge::Spline {
                degree,
                rational,
                periodic,
                knots,
                control_points,
                fit_points,
                start_tangent,
                end_tangent,
            } => BoundaryEdge::Spline {
                degree: *degree,
                rational: *rational,
                periodic: *periodic,
                knots: knots.clone(),
                control_points: control_points
                    .iter()
                    .map(|p| {
                        let mapped = map_point([p[0], p[1]]);
                        [mapped[0], mapped[1], p[2]]
                    })
                    .collect(),
                fit_points: fit_points.iter().map(|p| map_point(*p)).collect(),
                start_tangent: rotate(*start_tangent),
                end_tangent: rotate(*end_tangent),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests;
