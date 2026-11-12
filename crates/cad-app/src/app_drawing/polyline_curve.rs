//! The LWPOLYLINE parameter model shared by BREAK, BREAKATPOINT, DIVIDE, LENGTHEN
//! and TRIM.
//!
//! Split out of `divide.rs` so every module stays inside the code-size limit.
//! The parameter is `u ∈ [0, segments]`, one unit per segment.

use super::trim_curves::{cut_tolerance, norm_tau};
use super::*;
use cad_geometry::{arbitrary_axis, bulge_arc_geometry};

/// A polyline-layer refusal: the caller's command prefix plus the reason.
fn polyline_refusal(prefix: &str, code: &str, detail: impl std::fmt::Display) -> CadError {
    CadError::InvalidInput(format!("{prefix}.{code}: {detail}"))
}

/// A validated LWPOLYLINE parameter model shared by BREAK, BREAKATPOINT and
/// DIVIDE. Its parameter is `u ∈ [0, segments]`, one unit per segment.
pub(super) struct PolylineCurve {
    points: Vec<Point3>,
    /// One bulge per segment (`segments = points.len()` when closed,
    /// `points.len() - 1` when open).
    bulges: Vec<f64>,
    closed: bool,
    /// The refusal prefix of the calling command (`break`, `breakatpoint`,
    /// `divide`).
    prefix: &'static str,
}

impl PolylineCurve {
    /// Validate and normalise a stored polyline.
    pub(super) fn new(
        points: &[Point3],
        bulges: &[f64],
        closed: bool,
        prefix: &'static str,
    ) -> CadResult<Self> {
        let tol = cut_tolerance();
        let mut points = points.to_vec();
        let mut bulges = bulges.to_vec();
        if points.len() < 2 {
            return Err(polyline_refusal(
                prefix,
                "degenerate",
                "a polyline needs at least two vertices",
            ));
        }
        if points.iter().any(|point| !is_finite_point(*point)) {
            return Err(polyline_refusal(
                prefix,
                "invalid_point",
                "polyline vertices must be finite",
            ));
        }
        if closed && distance(points[0], *points.last().expect("checked non-empty")) <= tol {
            let original_len = points.len();
            points.pop();
            if bulges.len() == original_len {
                bulges.pop();
            }
        }
        if points.len() < 2 {
            return Err(polyline_refusal(
                prefix,
                "degenerate",
                "a closed polyline collapses to fewer than two distinct vertices",
            ));
        }
        let segments = if closed {
            points.len()
        } else {
            points.len() - 1
        };
        if bulges.len() < segments {
            if closed && bulges.len() == segments - 1 {
                bulges.push(0.0);
            } else {
                return Err(polyline_refusal(
                    prefix,
                    "unsupported",
                    format!(
                        "the polyline has {} vertices and {} bulges; one bulge per segment is \
                         required",
                        points.len(),
                        bulges.len()
                    ),
                ));
            }
        }
        bulges.truncate(segments);
        let has_bulge = bulges.iter().any(|bulge| bulge.abs() > 1e-9);
        if has_bulge && !cad_geometry::is_xy_parallel(&points) {
            return Err(CadError::Unsupported(format!(
                "{prefix}.bulged_segment: a bulged polyline must be +Z planar to resolve its \
                 arc segments"
            )));
        }
        Ok(Self {
            points,
            bulges,
            closed,
            prefix,
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed
    }

    pub(super) fn segment_count(&self) -> usize {
        if self.closed {
            self.points.len()
        } else {
            self.points.len() - 1
        }
    }

    fn bulge_at(&self, segment: usize) -> f64 {
        self.bulges.get(segment).copied().unwrap_or(0.0)
    }

    fn endpoints(&self, segment: usize) -> (Point3, Point3) {
        let start = self.points[segment];
        let end = if self.closed || segment + 1 < self.points.len() {
            self.points[(segment + 1) % self.points.len()]
        } else {
            self.points[segment + 1]
        };
        (start, end)
    }

    /// The point at the segment parameter `u` (closed curves wrap).
    pub(super) fn point_at(&self, u: f64) -> Point3 {
        let n = self.segment_count();
        if n == 0 {
            return self.points[0];
        }
        let u = if self.closed {
            u.rem_euclid(n as f64)
        } else {
            u.clamp(0.0, n as f64)
        };
        if !self.closed && u >= n as f64 {
            return *self.points.last().expect("validated non-empty");
        }
        let segment = (u.floor() as usize).min(n - 1);
        self.point_on_segment(segment, u - segment as f64)
            .unwrap_or_else(|_| self.points[segment])
    }

    /// The nearest point of one segment and its local `t ∈ [0, 1]`.
    fn project_on_segment(&self, segment: usize, point: Point3, tol: f64) -> Option<(f64, f64)> {
        let (a, b) = self.endpoints(segment);
        let bulge = self.bulge_at(segment);
        if bulge.abs() <= 1e-9 {
            let d = cad_geometry::sub(b, a);
            let len2 = cad_geometry::dot(d, d);
            if !len2.is_finite() || len2 <= 0.0 {
                return None;
            }
            let t = (cad_geometry::dot(cad_geometry::sub(point, a), d) / len2).clamp(0.0, 1.0);
            let closest = cad_geometry::add(a, cad_geometry::scale(d, t));
            return Some((t, cad_geometry::distance(closest, point)));
        }
        let SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } = bulge_arc_geometry(
            a,
            b,
            bulge,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )?
        else {
            return None;
        };
        if !radius.is_finite() || radius <= tol || sweep.abs() <= 1e-12 {
            return None;
        }
        let (ax, ay, _) = arbitrary_axis(normal);
        let v = cad_geometry::sub(point, center);
        let (rx, ry) = (cad_geometry::dot(v, ax), cad_geometry::dot(v, ay));
        if rx.hypot(ry) <= tol {
            return None;
        }
        let angle = ry.atan2(rx);
        let raw = if sweep >= 0.0 {
            norm_tau(angle - start)
        } else {
            -norm_tau(start - angle)
        };
        let t = (raw / sweep).clamp(0.0, 1.0);
        let at = start + sweep * t;
        let closest = cad_geometry::add(
            center,
            cad_geometry::add(
                cad_geometry::scale(ax, radius * at.cos()),
                cad_geometry::scale(ay, radius * at.sin()),
            ),
        );
        Some((t, cad_geometry::distance(closest, point)))
    }

    /// The parameter of the projection of `point` onto the nearest segment.
    pub(super) fn project_param(&self, point: Point3, tol: f64) -> CadResult<Option<f64>> {
        let mut best: Option<(usize, f64, f64)> = None;
        for segment in 0..self.segment_count() {
            if let Some((t, dist)) = self.project_on_segment(segment, point, tol) {
                if best.is_none_or(|(_, _, best_dist)| dist < best_dist) {
                    best = Some((segment, t, dist));
                }
            }
        }
        Ok(best.map(|(segment, t, _)| segment as f64 + t))
    }

    /// The total arc length of the polyline (exact for straight and circular
    /// bulged segments).
    pub(super) fn total_length(&self) -> f64 {
        (0..self.segment_count())
            .map(|segment| self.segment_length(segment))
            .sum()
    }

    /// One segment as the analytic geometry it really is: a LINE, or the exact
    /// ARC of a bulge (`bulge_arc_geometry`, the same construction
    /// [`Self::point_on_segment`] and [`Self::segment_length`] use).
    ///
    /// `None` for a segment whose two vertices coincide. A bulge too shallow to
    /// carry an arc is the chord, exactly like the boundary collector reads it, so
    /// a straight segment is never reported as an arc.
    pub(super) fn segment_geometry(&self, segment: usize) -> Option<SemanticGeometry> {
        let (a, b) = self.endpoints(segment);
        if distance(a, b) <= 1e-12 {
            return None;
        }
        let bulge = self.bulge_at(segment);
        if bulge.abs() <= 1e-9 {
            return Some(SemanticGeometry::Line { start: a, end: b });
        }
        Some(
            bulge_arc_geometry(
                a,
                b,
                bulge,
                Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
            )
            .unwrap_or(SemanticGeometry::Line { start: a, end: b }),
        )
    }

    /// The exact arc length of one segment.
    pub(super) fn segment_length(&self, segment: usize) -> f64 {
        let (a, b) = self.endpoints(segment);
        let bulge = self.bulge_at(segment);
        if bulge.abs() <= 1e-9 {
            return distance(a, b);
        }
        bulge_arc_geometry(
            a,
            b,
            bulge,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        .and_then(|geometry| match geometry {
            SemanticGeometry::Arc { radius, sweep, .. } => Some(radius * sweep.abs()),
            _ => None,
        })
        .unwrap_or_else(|| distance(a, b))
    }

    /// The point at `target` arc length from the start (closed curves wrap).
    pub(super) fn point_at_distance(&self, target: f64) -> Point3 {
        let total = self.total_length();
        if !target.is_finite() {
            return self.point_at(0.0);
        }
        let target = if self.closed {
            target.rem_euclid(total)
        } else {
            target.clamp(0.0, total)
        };
        let mut traversed = 0.0;
        for segment in 0..self.segment_count() {
            let length = self.segment_length(segment);
            if traversed + length >= target {
                let fraction = if length > 0.0 {
                    (target - traversed) / length
                } else {
                    0.0
                };
                return self
                    .point_on_segment(segment, fraction)
                    .unwrap_or_else(|_| self.point_at(segment as f64));
            }
            traversed += length;
        }
        self.point_at(self.segment_count() as f64)
    }

    /// The unit tangent direction angle (radians) at `target` arc length, or
    /// `None` when the polyline has no in-plane (XY) tangent there.
    ///
    /// Exact, not a difference quotient: a straight segment contributes its chord
    /// direction and a bulged segment the circular arc's tangent. At an exact
    /// interior vertex (the target lands on a segment boundary) the **outgoing**
    /// segment is chosen, so the value never straddles two segments (the old
    /// `point_at_distance(d ± h)` approximation turned a corner into a bisector
    /// and a long/narrow segment into a wrong direction).
    pub(super) fn tangent_angle_at_distance(&self, target: f64) -> Option<f64> {
        let total = self.total_length();
        if !total.is_finite() || total <= 0.0 || !target.is_finite() {
            return None;
        }
        let target = if self.closed {
            target.rem_euclid(total)
        } else {
            target.clamp(0.0, total)
        };
        let eps = 1e-9;
        let mut traversed = 0.0;
        let mut chosen = self.segment_count().saturating_sub(1);
        let mut local = 0.0;
        for segment in 0..self.segment_count() {
            let length = self.segment_length(segment);
            if target < traversed + length - eps || segment + 1 == self.segment_count() {
                chosen = segment;
                local = if length > 0.0 {
                    ((target - traversed) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                break;
            }
            traversed += length;
        }
        let (a, b) = self.endpoints(chosen);
        let bulge = self.bulge_at(chosen);
        if bulge.abs() <= 1e-9 {
            return in_plane_direction(cad_geometry::sub(b, a));
        }
        let SemanticGeometry::Arc {
            radius,
            start,
            sweep,
            normal,
            ..
        } = bulge_arc_geometry(
            a,
            b,
            bulge,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )?
        else {
            return None;
        };
        if !radius.is_finite() || radius <= 0.0 || sweep.abs() <= 1e-12 {
            return None;
        }
        let (ax, ay, _) = arbitrary_axis(normal);
        let angle = start + sweep * local;
        let tangent = cad_geometry::add(
            cad_geometry::scale(ax, -angle.sin()),
            cad_geometry::scale(ay, angle.cos()),
        );
        // The arc's own travel direction is sign(sweep) times the +t tangent.
        let tangent = if sweep < 0.0 {
            cad_geometry::scale(tangent, -1.0)
        } else {
            tangent
        };
        in_plane_direction(tangent)
    }

    /// The point at local segment parameter `t`.
    fn point_on_segment(&self, segment: usize, t: f64) -> CadResult<Point3> {
        let (a, b) = self.endpoints(segment);
        let bulge = self.bulge_at(segment);
        if bulge.abs() <= 1e-9 {
            let d = cad_geometry::sub(b, a);
            return Ok(cad_geometry::add(a, cad_geometry::scale(d, t)));
        }
        let SemanticGeometry::Arc {
            center,
            normal,
            radius,
            start,
            sweep,
        } = bulge_arc_geometry(
            a,
            b,
            bulge,
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
        )
        .ok_or_else(|| {
            polyline_refusal(
                self.prefix,
                "degenerate_piece",
                "a bulged segment could not be resolved as a circular arc",
            )
        })?
        else {
            return Err(polyline_refusal(
                self.prefix,
                "degenerate_piece",
                "a bulged segment did not resolve to an arc",
            ));
        };
        let (ax, ay, _) = arbitrary_axis(normal);
        let angle = start + sweep * t;
        Ok(cad_geometry::add(
            center,
            cad_geometry::add(
                cad_geometry::scale(ax, radius * angle.cos()),
                cad_geometry::scale(ay, radius * angle.sin()),
            ),
        ))
    }

    /// The open polyline on the forward span `[u_start, u_end]`.
    ///
    /// For a closed curve `u_start`/`u_end` may be unwrapped (`u_end` up to one
    /// full period past `u_start`); segment indices wrap.
    pub(super) fn sub_curve(&self, u_start: f64, u_end: f64) -> CadResult<SemanticGeometry> {
        let n = self.segment_count();
        let eps = 1e-12;
        if !u_start.is_finite() || !u_end.is_finite() || u_end - u_start <= eps {
            return Err(polyline_refusal(
                self.prefix,
                "degenerate_piece",
                "the kept polyline span is empty",
            ));
        }
        if self.closed {
            if u_end - u_start > n as f64 + eps {
                return Err(polyline_refusal(
                    self.prefix,
                    "degenerate_piece",
                    "the kept polyline span is longer than one full period",
                ));
            }
        } else if u_start < -eps || u_end > n as f64 + eps {
            return Err(polyline_refusal(
                self.prefix,
                "degenerate_piece",
                "the kept polyline span is outside the open polyline",
            ));
        }
        let start_index = u_start.floor().max(0.0) as usize;
        let end_inclusive = if (u_end - u_end.floor()).abs() <= eps {
            (u_end.ceil() as usize).saturating_sub(1)
        } else {
            u_end.floor() as usize
        };
        let mut points = Vec::new();
        let mut bulges = Vec::new();
        for global in start_index..=end_inclusive {
            let segment = if self.closed {
                global % n
            } else if global < n {
                global
            } else {
                break;
            };
            let lo = if global == start_index {
                (u_start - global as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let hi = if global == end_inclusive {
                (u_end - global as f64).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if hi - lo <= eps {
                continue;
            }
            let start = self.point_on_segment(segment, lo)?;
            let end = self.point_on_segment(segment, hi)?;
            if points.is_empty() {
                points.push(start);
            } else if distance(*points.last().expect("non-empty"), start) > cut_tolerance() {
                // A gap can only come from a degenerate skipped segment; keep the
                // chain continuous with a straight connector rather than losing it.
                points.push(start);
                bulges.push(0.0);
            }
            bulges.push(sub_bulge(self.bulge_at(segment), lo, hi));
            points.push(end);
        }
        if points.len() < 2 {
            return Err(polyline_refusal(
                self.prefix,
                "degenerate_piece",
                "the kept polyline piece has fewer than two vertices",
            ));
        }
        Ok(SemanticGeometry::Polyline {
            points,
            bulges,
            closed: false,
            width: 0.0,
        })
    }

    /// Split an **open** polyline at the projection of `point` (BREAKATPOINT).
    pub(super) fn split_at_point(
        &self,
        point: Point3,
        tol: f64,
    ) -> CadResult<(SemanticGeometry, SemanticGeometry)> {
        if self.closed {
            return Err(polyline_refusal(
                self.prefix,
                "closed",
                "BREAKATPOINT does not split a closed polyline",
            ));
        }
        let n = self.segment_count() as f64;
        let Some(u) = self.project_param(point, tol)? else {
            return Err(polyline_refusal(
                self.prefix,
                "projection_failed",
                "the point has no projection onto the polyline",
            ));
        };
        let total = self.total_length().max(tol);
        let param_tol = (tol / total).max(1e-12);
        if u <= param_tol || u >= n - param_tol {
            return Err(polyline_refusal(
                self.prefix,
                "point_at_endpoint",
                "the break point coincides with an endpoint of the polyline; there is no \
                 interior split to make",
            ));
        }
        Ok((self.sub_curve(0.0, u)?, self.sub_curve(u, n)?))
    }

    /// Insert one vertex on segment `segment` at its local parameter `t`,
    /// **without changing the curve's shape** (`PEDIT Edit vertex` → `Insert`).
    ///
    /// The inserted vertex is the exact point of the existing segment at `t`,
    /// and a bulged segment is split into the two sub-arcs of the same circle
    /// (`sub_bulge`), so a straight or bulged polyline keeps its geometry
    /// exactly. `t` must be strictly inside the segment; inserting on an
    /// existing vertex is refused by name.
    pub(super) fn insert_vertex_at(
        &self,
        segment: usize,
        t: f64,
    ) -> CadResult<(Vec<Point3>, Vec<f64>)> {
        if segment >= self.segment_count() {
            return Err(polyline_refusal(
                self.prefix,
                "vertex_out_of_range",
                format!(
                    "segment {segment} does not exist (the polyline has {} segments)",
                    self.segment_count()
                ),
            ));
        }
        if !t.is_finite() || t <= 1e-9 || t >= 1.0 - 1e-9 {
            return Err(polyline_refusal(
                self.prefix,
                "vertex_insert_at_vertex",
                "the insert point coincides with an existing vertex of the segment",
            ));
        }
        let point = self.point_on_segment(segment, t)?;
        let bulge = self.bulge_at(segment);
        let mut points = self.points.clone();
        let mut bulges = self.bulges.clone();
        points.insert(segment + 1, point);
        bulges[segment] = sub_bulge(bulge, 0.0, t);
        bulges.insert(segment + 1, sub_bulge(bulge, t, 1.0));
        Ok((points, bulges))
    }
}

/// The bulge of the sub-arc `[lo, hi]` of a segment with `bulge`.
fn sub_bulge(bulge: f64, lo: f64, hi: f64) -> f64 {
    if bulge.abs() <= 1e-9 {
        return 0.0;
    }
    let theta = 4.0 * bulge.atan();
    let sweep = theta * (hi - lo);
    if sweep.abs() <= 1e-12 {
        return 0.0;
    }
    cad_geometry::arc_sweep_to_bulge(sweep).unwrap_or(0.0)
}

/// The XY direction angle (radians) of `v`, or `None` when its in-plane
/// component vanishes (a purely vertical direction) or is non-finite.
fn in_plane_direction(v: Point3) -> Option<f64> {
    if !v.x.is_finite() || !v.y.is_finite() || v.x.hypot(v.y) <= 1e-12 {
        None
    } else {
        Some(v.y.atan2(v.x))
    }
}
