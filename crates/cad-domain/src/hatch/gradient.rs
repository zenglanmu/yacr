//! Gradient HATCH colouring (baked into per-vertex sRGB).

use super::fill::FillMesh;

// ---------------------------------------------------------------------------
// Gradient fills (spec §3.2 gradient HATCH)
// ---------------------------------------------------------------------------
//
// A gradient fill is baked into per-vertex sRGB so it reuses the existing mesh
// pipeline (no texture, no new shader state). The model below is intentionally
// a small, honest subset of the DXF gradient vocabulary:
//
//   * `Linear` — a 1D ramp along `angle` (DXF "LINEAR").
//   * `Spherical` — a radial ramp from the centre of the boundary extent
//     (DXF "SPHERICAL"/"CYLINDER" family, where the radius drives the value).
//
// Curved / multi-segment gradients (e.g. "HEMISPHERICAL", "CURVED") are not
// approximated: the importer reports them explicitly as unsupported.

/// One colour stop: `value` is the normalised position (`0.0..=1.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub value: f64,
    pub rgb: [u8; 3],
}

/// Which parametric ramp a gradient uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientKind {
    /// A 1D ramp along the gradient angle.
    Linear,
    /// A radial ramp from the centre of the boundary's extent.
    Spherical,
}

/// A resolved gradient definition in hatch-plane coordinates.
///
/// `angle` is the gradient direction in radians (DXF group 460), `shift` the
/// normalised start offset in `0.0..=1.0` (DXF group 461), and `stops` the
/// colour entries (DXF group 463/421). `single_color` records the DXF
/// single-colour flag; when set, `stops` is expected to hold one entry and the
/// ramp blends that colour towards `tint` (positive → white, negative → black).
#[derive(Debug, Clone, PartialEq)]
pub struct GradientDef {
    pub kind: GradientKind,
    pub angle: f64,
    pub shift: f64,
    pub single_color: bool,
    /// Tint in `-1.0..=1.0`; only meaningful for `single_color`.
    pub tint: f64,
    pub stops: Vec<GradientStop>,
}

impl GradientDef {
    /// Whether the definition carries the data needed to produce a ramp.
    ///
    /// A gradient with no usable stop cannot be rendered as a gradient and must
    /// be reported rather than silently drawn solid.
    pub fn is_usable(&self) -> bool {
        !self.stops.is_empty()
            && self.angle.is_finite()
            && self.shift.is_finite()
            && self
                .stops
                .iter()
                .all(|s| s.value.is_finite() && s.value >= 0.0 && s.value <= 1.0)
    }
}

/// Sample a gradient at normalised position `t` (clamped to `0.0..=1.0`).
///
/// Stops must be sorted by `value` via [`normalize_stops`]; a single stop is a
/// constant colour. Between two stops the colour is linearly interpolated in
/// sRGB byte space, which matches the no-colour-management path the renderer
/// already uses.
pub fn sample_gradient(def: &GradientDef, t: f64) -> [u8; 3] {
    let stops = &def.stops;
    if stops.is_empty() {
        return [255, 255, 255];
    }
    let t = t.clamp(0.0, 1.0);
    if stops.len() == 1 {
        return stops[0].rgb;
    }
    if t <= stops[0].value {
        return stops[0].rgb;
    }
    if t >= stops[stops.len() - 1].value {
        return stops[stops.len() - 1].rgb;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t >= a.value && t <= b.value {
            let span = b.value - a.value;
            if span <= f64::EPSILON {
                return b.rgb;
            }
            let f = (t - a.value) / span;
            return lerp_rgb(a.rgb, b.rgb, f);
        }
    }
    stops[stops.len() - 1].rgb
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], f: f64) -> [u8; 3] {
    let f = f.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| {
        (x as f64 + (y as f64 - x as f64) * f)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]
}

/// Sort stops ascending and drop non-finite / out-of-range entries.
///
/// Keeps the gradient deterministic even when a file lists stops out of order,
/// which is common. Duplicate values are retained; the sampler resolves a
/// zero-width span to the later stop.
pub fn normalize_stops(mut stops: Vec<GradientStop>) -> Vec<GradientStop> {
    stops.retain(|s| s.value.is_finite() && (0.0..=1.0).contains(&s.value));
    stops.sort_by(|a, b| {
        a.value
            .partial_cmp(&b.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    stops
}

/// The parameter each vertex feeds into the ramp, in `0.0..=1.0`.
///
/// * Linear: projection onto the gradient direction, normalised across the
///   boundary extent, then shifted by `shift` (wrapping, as DXF does).
/// * Spherical: normalised radial distance from the centre of the extent.
fn gradient_parameter(points: &[[f64; 2]], def: &GradientDef) -> Vec<f64> {
    let Some((min, max)) = bounds_of(points) else {
        return Vec::new();
    };
    let center = [0.5 * (min[0] + max[0]), 0.5 * (min[1] + max[1])];
    match def.kind {
        GradientKind::Linear => {
            let (dx, dy) = (def.angle.cos(), def.angle.sin());
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for p in points {
                let s = (p[0] - center[0]) * dx + (p[1] - center[1]) * dy;
                lo = lo.min(s);
                hi = hi.max(s);
            }
            let span = hi - lo;
            points
                .iter()
                .map(|p| {
                    let s = (p[0] - center[0]) * dx + (p[1] - center[1]) * dy;
                    let t = if span <= f64::EPSILON {
                        0.5
                    } else {
                        (s - lo) / span
                    };
                    // The shift is an offset along the ramp. This subset clamps
                    // at the ends rather than wrapping, so the two boundary
                    // endpoints keep their exact stop colours; wrapping is a
                    // documented gap (see docs/hatch-gradient.md).
                    (t + def.shift).clamp(0.0, 1.0)
                })
                .collect()
        }
        GradientKind::Spherical => {
            let mut max_r = 0.0f64;
            for p in points {
                let dx = p[0] - center[0];
                let dy = p[1] - center[1];
                max_r = max_r.max((dx * dx + dy * dy).sqrt());
            }
            points
                .iter()
                .map(|p| {
                    let dx = p[0] - center[0];
                    let dy = p[1] - center[1];
                    let r = (dx * dx + dy * dy).sqrt();
                    let t = if max_r <= f64::EPSILON {
                        0.0
                    } else {
                        r / max_r
                    };
                    (t + def.shift).clamp(0.0, 1.0)
                })
                .collect()
        }
    }
}

fn bounds_of(points: &[[f64; 2]]) -> Option<([f64; 2], [f64; 2])> {
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    let mut any = false;
    for p in points {
        any = true;
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    any.then_some((min, max))
}

/// Bake a gradient into per-vertex sRGB for a fill triangulation.
///
/// Each returned colour corresponds to `fill.vertices[i]`, so the existing
/// fill indices apply unchanged and the ramp is clipped to the hatch boundary
/// by construction (only interior vertices are coloured). Returns an empty
/// vector when the definition is unusable, which the caller must report rather
/// than draw solid.
pub fn gradient_vertex_colors(fill: &FillMesh, def: &GradientDef) -> Vec<[u8; 3]> {
    if !def.is_usable() {
        return Vec::new();
    }
    let stops = normalize_stops(def.stops.clone());
    if stops.is_empty() {
        return Vec::new();
    }
    let effective = if def.single_color {
        single_color_stops(&stops, def.tint)
    } else {
        stops
    };
    let def = GradientDef {
        stops: effective,
        ..def.clone()
    };
    let params = gradient_parameter(&fill.vertices, &def);
    params.iter().map(|t| sample_gradient(&def, *t)).collect()
}

/// Expand a DXF single-colour gradient into a two-stop ramp.
///
/// The single listed colour stays at value 0; value 1 blends it towards white
/// (`tint > 0`) or black (`tint < 0`), mirroring AutoCAD's tint/shade control.
fn single_color_stops(stops: &[GradientStop], tint: f64) -> Vec<GradientStop> {
    let base = stops[0];
    let tint = tint.clamp(-1.0, 1.0);
    let target = if tint >= 0.0 {
        [255, 255, 255]
    } else {
        [0, 0, 0]
    };
    let end = lerp_rgb(base.rgb, target, tint.abs());
    vec![
        GradientStop {
            value: 0.0,
            rgb: base.rgb,
        },
        GradientStop {
            value: 1.0,
            rgb: end,
        },
    ]
}
