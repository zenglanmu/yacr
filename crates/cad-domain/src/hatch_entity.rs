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
//! style, the pattern-definition composite and the world placement). The pattern
//! and gradient definition types live in [`fill`] (re-exported here, so every
//! public path is unchanged).

use crate::hatch::{
    fill_rings, gradient_vertex_colors, pattern_polylines, simplify, FillError, PatternLine,
};
use crate::{CadError, CadResult, Mesh, Point3, SemanticGeometry, Transform3};

use geometry::{add3, dot3, length3, scale3, sub3};
use geometry::{
    arbitrary_axis, column_scale, fill_tolerance, finite_point, normalize3, sample_loop,
};
use validate::{loop_depths, plane_is_usable, validate_creation};

mod fill;
mod geometry;
// The persisted restore constructor (ADR-0037 D8), split out for the code-size
// gate; a child module can reach the private fields without widening them.
mod persist;
mod validate;

pub use fill::{HatchFill, HatchGradient, HatchPattern, HatchPatternLine};

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
    /// A text island: the fill hole a text-shaped island carves out.
    pub const TEXT_ISLAND: u32 = 128;

    /// The raw bits (DXF group 92).
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Whether the loop is a derived boundary; only such a hatch writes group
    /// 47 (ADR B3).
    pub fn derived(self) -> bool {
        self.0 & Self::DERIVED != 0
    }

    /// Whether the loop is a text box (DXF group 92 bit 8).
    pub fn textbox(self) -> bool {
        self.0 & Self::TEXTBOX != 0
    }

    /// Whether the loop is a text island (DXF group 92 bit 128).
    pub fn text_island(self) -> bool {
        self.0 & Self::TEXT_ISLAND != 0
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

    /// Whether this loop is a **fill island the drawing does not show**.
    ///
    /// A text box (`92` bit 8) or a text island (`92` bit 128) is a hole the
    /// hatch engine carved around text, not boundary geometry the author drew:
    /// the vendor drawing shows no outline there. Drawing it adds visible
    /// rectangles inside filled areas, so the *display* skips it while the loop
    /// itself is kept — the fill parity, the snap vertices and the export all
    /// still see it (ADR 0018 §B.7).
    pub fn is_text_island(&self) -> bool {
        self.flags.textbox() || self.flags.text_island()
    }

    /// Sample this loop's bulges into a dense closed 2D ring.
    ///
    /// The returned ring has no repeated last point and may contain more
    /// vertices than [`Self::points`] when the loop carries bulges. It delegates
    /// to the same sampler the fill/display derivation uses
    /// (`hatch_entity::geometry::sample_loop`), so a caller never has to
    /// re-implement the DXF bulge convention (circle bulge tests in `cad-app`
    /// rely on this staying a single implementation).
    pub fn sampled_ring(&self) -> Vec<[f64; 2]> {
        sample_loop(self)
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
///
/// `pattern_origin` is the pattern's **phase displacement**: a hatch-plane 2D
/// vector added to every pattern family's base point before the even-odd clip,
/// so it moves the pattern lines *and* their dash phase. Neither the DXF nor the
/// DWG hatch record has a dedicated pattern-origin group; both store the phase
/// in each definition row's base point (groups 43/44), which is what AutoCAD's
/// HATCHEDIT ORIGIN rewrites. This field is therefore the displacement from the
/// stored rows: `[0.0, 0.0]` means "the file's own origin", and the export folds
/// a non-zero value back into the rows (see `cad-connector-acadrust`'s
/// `export::hatch`), never into a field no consumer would read.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchGeometry {
    loops: Vec<HatchLoop>,
    plane: HatchPlane,
    style: HatchStyle,
    fill: HatchFill,
    seeds: Vec<[f64; 2]>,
    pixel_size: Option<f64>,
    pattern_type: HatchPatternType,
    pattern_origin: [f64; 2],
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
            // A freshly created hatch has no phase displacement: the rows the
            // caller supplies are the definition, so the pattern starts at their
            // own origin.
            pattern_origin: [0.0, 0.0],
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
            // The source rows already carry the file's pattern phase (there is no
            // separate origin group), so an imported hatch needs no displacement.
            pattern_origin: [0.0, 0.0],
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
            // `angle_raw` is a stored radian value (group 460) and a direction in
            // this plane, so it follows the same `delta` as the pattern angle:
            // the hatch turns and the ramp turns with it. It is never converted
            // to/from degrees.
            HatchFill::Gradient(gradient) => HatchFill::Gradient(gradient.rotated(delta)),
        };
        let mut hatch = HatchGeometry {
            loops,
            plane: new_plane,
            style: self.style,
            fill,
            seeds,
            pixel_size: self.pixel_size,
            pattern_type: self.pattern_type,
            // The displacement is a vector in the old plane frame, so it follows
            // the similarity's induced in-plane rotation and scale — the same
            // `delta`/`scale` the pattern angle and scale above use. Otherwise a
            // moved hatch would keep its pattern phase in the old frame.
            pattern_origin: {
                let (sin, cos) = delta.sin_cos();
                let x = self.pattern_origin[0] * scale;
                let y = self.pattern_origin[1] * scale;
                [cos * x - sin * y, sin * x + cos * y]
            },
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

    /// Replace the hatch style/fill/pattern type and rebuild the derived display.
    ///
    /// The boundary loops, plane (**including its origin/elevation**), seeds and
    /// pixel size are untouched, so a property painter can copy the pattern
    /// definition without moving the hatch. Degradations that prevent a fill are
    /// handled exactly like [`Self::transformed`]: the boundary display is still
    /// built and the reasons are not promoted to a command failure.
    pub fn with_style_and_fill(
        &self,
        style: HatchStyle,
        fill: HatchFill,
        pattern_type: HatchPatternType,
    ) -> HatchGeometry {
        let mut hatch = HatchGeometry {
            loops: self.loops.clone(),
            plane: self.plane,
            style,
            fill,
            seeds: self.seeds.clone(),
            pixel_size: self.pixel_size,
            pattern_type,
            pattern_origin: self.pattern_origin,
            display: Vec::new(),
        };
        hatch.display = hatch.build_display().0;
        hatch
    }

    /// Move the pattern origin (a hatch-plane displacement of the pattern's
    /// definition rows) and rebuild the derived display.
    ///
    /// The value is the displacement from the stored rows — see the type docs for
    /// why the model keeps it that way. Only a [`HatchFill::Pattern`] draws rows,
    /// so a solid/gradient hatch stores the value without a visible change; the
    /// export declares it rather than silently dropping it.
    pub fn with_pattern_origin(&self, origin: [f64; 2]) -> HatchGeometry {
        let mut hatch = HatchGeometry {
            loops: self.loops.clone(),
            plane: self.plane,
            style: self.style,
            fill: self.fill.clone(),
            seeds: self.seeds.clone(),
            pixel_size: self.pixel_size,
            pattern_type: self.pattern_type,
            pattern_origin: origin,
            display: Vec::new(),
        };
        hatch.display = hatch.build_display().0;
        hatch
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

    /// The pattern phase displacement in the hatch-plane frame (see the type
    /// docs: this is the offset from the stored definition rows, so `[0.0, 0.0]`
    /// is the source file's own origin).
    pub fn pattern_origin(&self) -> [f64; 2] {
        self.pattern_origin
    }

    /// The pattern's **absolute origin** in the hatch-plane frame: where the
    /// composed base point of the first definition row currently sits.
    ///
    /// [`HatchPattern::families`] composes the stored rows into the plane frame
    /// (the hatch-level angle and scale applied to each row's base point) and
    /// [`Self::pattern_origin`] then displaces every family, so the place the
    /// pattern's own phase reference occupies is
    /// `families()[0].base + pattern_origin`. That sum — and nothing else — is
    /// the absolute origin this model can name exactly: there is no separate
    /// origin record, because neither the DXF nor the DWG hatch record stores
    /// one (the file's phase lives in the rows).
    ///
    /// The **first** row is the reference; a pattern with several rows keeps each
    /// row's stored offset from it, so moving this point moves the whole pattern
    /// rigidly (the dash phases and the `double` family's copy included). A
    /// pattern that stores no rows (a user-defined pattern carrying only a
    /// spacing has no base point at all) and a non-pattern fill both return
    /// `None` rather than inventing an origin.
    pub fn absolute_pattern_origin(&self) -> Option<[f64; 2]> {
        let HatchFill::Pattern(pattern) = &self.fill else {
            return None;
        };
        let base = pattern.families().first().map(|line| line.base)?;
        Some([
            base[0] + self.pattern_origin[0],
            base[1] + self.pattern_origin[1],
        ])
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
    ///
    /// Text-box / text-island loops are not drawn: they are fill holes, and the
    /// vendor drawing shows no outline for them (see
    /// [`HatchLoop::is_text_island`]).
    fn boundary_display(&self) -> Vec<SemanticGeometry> {
        self.loops
            .iter()
            .filter(|loop_| !loop_.is_text_island())
            .map(|loop_| SemanticGeometry::Polyline {
                points: loop_
                    .points
                    .iter()
                    .map(|p| self.plane.to_world(*p))
                    .collect(),
                bulges: loop_.bulges.clone(),
                closed: true,
                width: 0.0,
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
                let families = self.pattern_families(pattern);
                if families.is_empty() {
                    reasons.push("hatch pattern has no line families".to_string());
                } else {
                    for line in pattern_polylines(&sampled, &families) {
                        if line.len() >= 2 {
                            display.push(SemanticGeometry::Polyline {
                                points: line.iter().map(|p| self.plane.to_world(*p)).collect(),
                                bulges: Vec::new(),
                                closed: false,
                                width: 0.0,
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

    /// The pattern line families in the hatch-plane frame with the stored
    /// pattern-origin displacement applied.
    ///
    /// [`HatchPattern::families`] composes the raw definition rows into the plane
    /// frame (`R(angle) · scale · row`); the displacement is added to each
    /// family's base point afterwards, which moves both the family's `k = 0` line
    /// and its dash phase origin, exactly like moving a pattern's base point in
    /// AutoCAD. An empty family list stays empty (the caller reports it).
    fn pattern_families(&self, pattern: &HatchPattern) -> Vec<PatternLine> {
        let [dx, dy] = self.pattern_origin;
        let mut families = pattern.families();
        if dx != 0.0 || dy != 0.0 {
            for family in &mut families {
                family.base = [family.base[0] + dx, family.base[1] + dy];
            }
        }
        families
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
            // The gradient is baked into per-vertex colours; a hatch fill has no
            // texture parameterisation.
            uvs: Vec::new(),
            texture: None,
            // A hatch fill declares no PBR material (ADR-0039 D1).
            pbr: None,
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
#[cfg(test)]
mod tests_origin;
