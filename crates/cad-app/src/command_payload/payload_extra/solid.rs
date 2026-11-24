//! The typed construction inputs of the seven 3D solid primitive commands.
//!
//! `BOX`, `CONE`, `CYLINDER`, `SPHERE`, `TORUS`, `WEDGE` and `PYRAMID` are the
//! one command family whose AutoCAD prompts take **dimensions** as well as
//! points. This carrier has no interactive prompt (ADR-0036 C1; the prompt
//! capture itself is a declared global-scope carve-out), so every construction
//! the prompts reach is expressed here as one typed value instead. The payload
//! carries only the input; `cad-app` maps it onto the modeler seam
//! (`crate::app_drawing::solid_spec`), which is the single place where the
//! geometry rules and their refusals live.
//!
//! # Plan coordinates and `ELEV`
//!
//! AutoCAD's primitive prompts are **plan picks**: a point typed as `x,y` takes
//! its z from the current elevation (`ELEV`), while a point typed with all three
//! coordinates keeps that explicit z. That rule is mirrored exactly:
//!
//! * [`SolidPrimitiveCommand::Points`] is the explicit 3D form (existing
//!   contract) — every coordinate is the caller's, so `ELEV` never shifts it;
//! * every other variant takes its base on the work plane as `[f64; 2]` (x, y)
//!   and the base z is the session `ELEV` token. This is the one consumer of
//!   `ELEV` for explicit 3D solids, and it is why a `BOX LENGTH` at `ELEV 10`
//!   lands on `z = 10` while `BOX` from two 3D corners stays where they say.
//!
//! Two option forms carry **explicit 3D** points and therefore ignore `ELEV`,
//! because AutoCAD's own prompt takes them in 3D:
//! [`SolidPrimitiveCommand::CenterPoints`] (the `Center` prompt's corner must
//! have a z for the body to be centred in height too) and
//! [`SolidPrimitiveCommand::SphereTwoPoint`] (SPHERE's `2P` is documented as two
//! points **anywhere in 3D space** defining a diameter).
//!
//! # Why there is no `BOX CUBE` / `CONE DIAMETER` command selector
//!
//! The repository's other construction options (`CIRCLE 2P`, `POLYGON
//! CIRCUMSCRIBED`) are canonical multi-word selectors because each one starts its
//! own **capture tool**. The solid family has no capture (the per-option prompts
//! are out of scope), so a selector that could only ever arrive without a payload
//! would be a fake surface. The typed payload is therefore the whole supported
//! surface, exactly as `Points([..])` already was. The one exception is
//! `PYRAMID`, whose *base* is a whole construction (side count + centre + radius +
//! inscribed/circumscribed) rather than a single field, so the session admits it
//! as the typed command line `PYRAMID [INSCRIBED|CIRCUMSCRIBED] <sides> <cx> <cy>
//! <radius> <height>` and dispatches this payload — still no second command id
//! and no second write path.

use cad_domain::Point3;

/// A circle on the work plane, as the AutoCAD construction prompts define it.
///
/// `center`/`first`/`second` are plan coordinates (x, y); the plane's z is the
/// session `ELEV` token. `radius`/`diameter` are drawing units and must be
/// finite and strictly positive. `TwoPoint` supplies the two ends of a diameter
/// — the base-circle construction CYLINDER/CONE call `2P` and TORUS uses for its
/// ring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanCircle {
    /// `center` + `radius` (AutoCAD's plain `Specify base radius:` prompt).
    Radius { center: [f64; 2], radius: f64 },
    /// `center` + `diameter` (`Diameter` at the radius prompt).
    Diameter { center: [f64; 2], diameter: f64 },
    /// Two ends of a diameter (`2P` at the centre prompt).
    TwoPoint { first: [f64; 2], second: [f64; 2] },
}

/// A length on the work plane — TORUS's tube input.
///
/// TORUS asks for the tube radius (`Specify tube radius or [2Point/Diameter]`),
/// so the same three spellings as [`PlanCircle`]'s radius are carried here. The
/// `TwoPoint` variant's tube radius is the **plan distance** between the two
/// points (AutoCAD's `2Point` at that prompt defines the tube by two points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanLength {
    /// The tube radius itself.
    Radius(f64),
    /// The tube diameter (`Diameter` at the tube prompt).
    Diameter(f64),
    /// Two plan points whose distance is the tube radius (`2Point`).
    TwoPoint { first: [f64; 2], second: [f64; 2] },
}

/// One typed construction of a 3D solid primitive.
///
/// The kind comes from the [`crate::CommandId`] (`BOX`, `CONE`, …); this value
/// only describes how its dimensions were supplied. `cad-app` rejects a
/// construction a kind does not take (for example `Cube` on `SPHERE`) with the
/// stable `solid.option_not_supported: <KIND> does not take the <option>
/// construction` refusal, before the modeler is called.
///
/// `BOX` and `WEDGE` share their whole prompt set (first corner or `Center`;
/// other corner, `Cube` or `Length`; height), so they share these variants.
/// `PYRAMID` takes [`Self::Points`] (the explicit square-base form) and
/// [`Self::PyramidPolygon`] (the typed regular n-gon base, AutoCAD's `Sides` +
/// `Inscribed`/`Circumscribed` construction); its `Top radius` truncation is not
/// modelled because the modeler seam has no truncated pyramid spec — it is
/// refused by absence, never folded into a full pyramid.
#[derive(Debug, Clone, PartialEq)]
pub enum SolidPrimitiveCommand {
    /// The explicit 3D form: every point is the caller's (arity per kind).
    ///
    /// This is the pre-existing contract, unchanged: a full 3D point keeps its
    /// own z, so `ELEV` never translates it.
    Points(Vec<Point3>),

    /// `BOX`/`WEDGE` `Cube`: a base corner on the plan plus one edge length;
    /// length, width and height are all `edge`.
    Cube { corner: [f64; 2], edge: f64 },
    /// `BOX`/`WEDGE` `Center` then a specific corner: the body is centred on
    /// `center` in **x, y and z** (AutoCAD's "centred on the point, in height
    /// too"), so `corner`'s mirror image about `center` is the opposite corner
    /// and each extent is twice the corner offset.
    CenterPoints { center: Point3, corner: Point3 },
    /// `BOX`/`WEDGE` `Center` then `Cube`: a cube centred on the plan point on
    /// all three axes.
    CenterCube { center: [f64; 2], edge: f64 },
    /// `BOX`/`WEDGE` `Center` then `Length`: extents from a plan centre point,
    /// centred on all three axes.
    CenterLengths {
        center: [f64; 2],
        length: f64,
        width: f64,
        height: f64,
    },
    /// `BOX`/`WEDGE` `Length`: a plan base corner plus length (x), width (y) and
    /// height (z).
    Length {
        corner: [f64; 2],
        length: f64,
        width: f64,
        height: f64,
    },
    /// `CYLINDER`: a plan base circle plus height.
    CylinderPlan { base: PlanCircle, height: f64 },
    /// `CONE`: a plan base circle, the `Top radius` field and the height.
    ///
    /// `top_radius == 0.0` is AutoCAD's default (a pointed cone); any other
    /// value is the `Top radius` frustum construction. A top radius equal to the
    /// base radius is refused as `cone.equal_radii` (that is a `CYLINDER`).
    ConePlan {
        base: PlanCircle,
        top_radius: f64,
        height: f64,
    },
    /// `SPHERE` `Diameter`: a plan centre plus the diameter.
    SphereDiameter { center: [f64; 2], diameter: f64 },
    /// `SPHERE` `2P`: two points **anywhere in 3D** are the ends of a diameter.
    ///
    /// AutoCAD defines the sphere's circumference (a great circle) through them,
    /// so the centre is their midpoint and the radius half their distance. The
    /// explicit 3D points keep their own coordinates; `ELEV` does not apply.
    SphereTwoPoint { first: Point3, second: Point3 },
    /// `TORUS`: a plan ring plus its tube.
    TorusPlan { ring: PlanCircle, tube: PlanLength },
    /// `PYRAMID`'s **regular n-gon base** construction:
    /// `PYRAMID [CIRCUMSCRIBED] <sides> <cx> <cy> <radius> <height>`.
    ///
    /// `base` is the plan centre (its z is the session `ELEV`), `radius` is the
    /// typed base radius and `circumscribed` selects which circle it names —
    /// exactly like `POLYGON`/`POLYGON CIRCUMSCRIBED`: `false` (the default)
    /// inscribes the base ring in the radius circle (the first vertex lies on
    /// `+X`), `true` has the ring's edges touch a radius circle (the extracted
    /// ring is `POLYGON CIRCUMSCRIBED`'s). The command converts the typed radius
    /// to the ring's vertex radius with the same polygon math the `POLYGON`
    /// command uses, so the two commands agree on the base vertices.
    ///
    /// The apex stays over the centre; the result is the same exact
    /// `Body3d { kind: Solid }` / `AcDb3dSolid` the explicit point form creates.
    /// AutoCAD's `Top radius` (truncated pyramid) is **not** modelled: the seam
    /// has no truncated spec, so it is absent rather than silently squared.
    PyramidPolygon {
        base: [f64; 2],
        radius: f64,
        sides: u32,
        circumscribed: bool,
        height: f64,
    },
}
