//! Derived vendor geometry: measurement by source handle.
//!
//! A TCH entity is opaque in `cad-db`; its display geometry lives in the settled
//! vendor snapshot keyed by the entity's source handle. This module turns the
//! snapshot's straight and circular edges into one closed planar ring and reuses
//! the same analytic area rules as `AREA OBJECT`:
//!
//! * a ring of straight edges (`Lines` / `Segments`) → shoelace area;
//! * circular arcs (`Arc`) → chord polygon + the exact circular-segment term
//!   `½ r² (θ − sin θ)` per edge, with the term negated when the ring traversal
//!   reverses that edge;
//! * a single `Circle` → `π r²`.
//!
//! Text and block-insert primitives are **not** measured (a text glyph is not a
//! ring; an inserted symbol would need block expansion, which this path does not
//! perform). Anything that does not form exactly one closed, planar ring is an
//! explicit refusal, never a partial or sampled number.

use cad_db::DbEntity;
use cad_domain::{CadError, CadResult, Point3, WorkPlane};

use crate::app_measure_object::MeasuredObjectArea;
use crate::vendor::VendorPrimitive;
use crate::Application;

use std::collections::HashMap;

/// One directed-in-construction edge of the derived ring.
struct DerivedEdge {
    start: Point3,
    end: Point3,
    /// Signed circular-segment term; `0` for a straight edge.
    segment: f64,
}

impl Application {
    /// Measure one vendor-derived entity from its settled primitives.
    ///
    /// `None` means the entity has no settled derived primitives (fall through
    /// to the normal unsupported-kind refusal). `Some(Err(..))` is an explicit
    /// refusal of the derived outline.
    pub(crate) fn measure_vendor_area(
        &self,
        entity: &DbEntity,
    ) -> Option<CadResult<MeasuredObjectArea>> {
        let store = self.vendor_representations.as_deref()?;
        let handle = entity.object.source_handle.as_deref()?;
        let primitives = store.for_source_handle(handle)?;
        Some(measure_derived_ring(entity, primitives))
    }
}

/// Build one closed planar ring from the primitives and compute its area.
fn measure_derived_ring(
    entity: &DbEntity,
    primitives: &[VendorPrimitive],
) -> CadResult<MeasuredObjectArea> {
    // A lone circle is exact without chaining.
    if primitives.len() == 1 {
        if let VendorPrimitive::Circle { center, radius } = &primitives[0] {
            if !center.x.is_finite()
                || !center.y.is_finite()
                || !radius.is_finite()
                || *radius <= 0.0
            {
                return Err(derived_refusal(entity, "the derived circle is degenerate"));
            }
            let value = std::f64::consts::PI * radius * radius;
            if !value.is_finite() || value <= 0.0 {
                return Err(derived_refusal(
                    entity,
                    "the derived circle has zero or non-finite area",
                ));
            }
            return Ok(MeasuredObjectArea {
                derived: true,
                kind: "vendor-derived circle",
                value,
                inputs: Vec::new(),
                plane: None,
            });
        }
    }

    let mut edges: Vec<DerivedEdge> = Vec::new();
    for primitive in primitives {
        match primitive {
            VendorPrimitive::Lines(points) => push_polyline(points, &mut edges),
            VendorPrimitive::Segments(points) => push_polyline(points, &mut edges),
            VendorPrimitive::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                if !center.x.is_finite()
                    || !center.y.is_finite()
                    || !radius.is_finite()
                    || *radius <= 0.0
                    || !start.is_finite()
                    || !sweep.is_finite()
                    || *sweep == 0.0
                {
                    return Err(derived_refusal(
                        entity,
                        "a derived arc has non-finite or degenerate parameters",
                    ));
                }
                let end_angle = start + sweep;
                edges.push(DerivedEdge {
                    start: Point3 {
                        x: center.x + radius * start.cos(),
                        y: center.y + radius * start.sin(),
                        z: center.z,
                    },
                    end: Point3 {
                        x: center.x + radius * end_angle.cos(),
                        y: center.y + radius * end_angle.sin(),
                        z: center.z,
                    },
                    segment: 0.5 * radius * radius * (sweep - sweep.sin()),
                });
            }
            VendorPrimitive::Text(_) => {
                return Err(derived_refusal(
                    entity,
                    "the derived outline contains text; text is not a measurable ring",
                ));
            }
            VendorPrimitive::Block { .. } => {
                return Err(derived_refusal(
                    entity,
                    "the derived outline references a symbol block; block expansion is not \
                     measured on this path",
                ));
            }
            VendorPrimitive::Circle { .. } => {
                return Err(derived_refusal(
                    entity,
                    "the derived outline mixes a full circle with other edges",
                ));
            }
        }
    }
    if edges.is_empty() {
        return Err(derived_refusal(
            entity,
            "the vendor snapshot has no measurable edges",
        ));
    }

    let ring = chain_ring(entity, &edges)?;
    let z = edges[0].start.z;
    for edge in &edges {
        if (edge.start.z - z).abs() > 1e-6 || (edge.end.z - z).abs() > 1e-6 {
            return Err(derived_refusal(
                entity,
                "the derived outline is not planar in Z",
            ));
        }
    }

    let mut cross = 0.0;
    let mut segments = 0.0;
    for index in &ring.order {
        let (edge, forward) = (&edges[*index], ring.forward[*index]);
        let start = if forward { edge.start } else { edge.end };
        let end = if forward { edge.end } else { edge.start };
        cross += start.x * end.y - end.x * start.y;
        // The circular-segment term follows the traversal direction.
        segments += if forward { edge.segment } else { -edge.segment };
    }
    let value = (cross * 0.5 + segments).abs();
    if !value.is_finite() || value <= 0.0 {
        return Err(derived_refusal(
            entity,
            "the derived ring has zero or non-finite area",
        ));
    }
    let inputs: Vec<Point3> = ring.order.iter().map(|index| edges[*index].start).collect();
    Ok(MeasuredObjectArea {
        derived: true,
        kind: "vendor-derived closed outline",
        value,
        inputs,
        plane: Some(WorkPlane {
            origin: Point3 { x: 0.0, y: 0.0, z },
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
        }),
    })
}

fn push_polyline(points: &[Point3], edges: &mut Vec<DerivedEdge>) {
    for pair in points.windows(2) {
        edges.push(DerivedEdge {
            start: pair[0],
            end: pair[1],
            segment: 0.0,
        });
    }
}

struct ChainedRing {
    order: Vec<usize>,
    /// Whether each edge is traversed start→end (`true`) or end→start.
    forward: Vec<bool>,
}

/// Walk the edge set into exactly one closed ring, refusing anything else.
fn chain_ring(entity: &DbEntity, edges: &[DerivedEdge]) -> CadResult<ChainedRing> {
    const SCALE: f64 = 1e7;
    let key = |point: Point3| -> (i64, i64) {
        (
            (point.x * SCALE).round() as i64,
            (point.y * SCALE).round() as i64,
        )
    };
    let mut incident: HashMap<(i64, i64), Vec<(usize, bool)>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        incident
            .entry(key(edge.start))
            .or_default()
            .push((index, true));
        incident
            .entry(key(edge.end))
            .or_default()
            .push((index, false));
    }
    if incident.values().any(|list| list.len() != 2) {
        return Err(derived_refusal(
            entity,
            "the derived edges do not form one closed ring (a vertex is not shared by two edges)",
        ));
    }

    let mut used = vec![false; edges.len()];
    let mut order = Vec::with_capacity(edges.len());
    let mut forward = Vec::with_capacity(edges.len());
    // Start with edge 0 traversed start→end.
    let mut current = 0usize;
    let mut current_forward = true;
    let start_key = key(edges[0].start);
    for _ in 0..edges.len() {
        if used[current] {
            return Err(derived_refusal(entity, "the derived ring repeats an edge"));
        }
        used[current] = true;
        order.push(current);
        forward.push(current_forward);
        let tail = if current_forward {
            edges[current].end
        } else {
            edges[current].start
        };
        let tail_key = key(tail);
        let Some(candidates) = incident.get(&tail_key) else {
            return Err(derived_refusal(entity, "the derived ring is open"));
        };
        let Some(&(next, next_forward)) = candidates.iter().find(|(index, _)| !used[*index]) else {
            // No unused edge: this must be the closing step.
            if tail_key == start_key && order.len() == edges.len() {
                return Ok(ChainedRing { order, forward });
            }
            return Err(derived_refusal(entity, "the derived ring does not close"));
        };
        current = next;
        current_forward = next_forward;
    }
    if tail_end(edges, &order, &forward) == start_key {
        Ok(ChainedRing { order, forward })
    } else {
        Err(derived_refusal(entity, "the derived ring does not close"))
    }
}

fn tail_end(edges: &[DerivedEdge], order: &[usize], forward: &[bool]) -> (i64, i64) {
    const SCALE: f64 = 1e7;
    let last = *order.last().expect("non-empty ring");
    let point = if forward[order.len() - 1] {
        edges[last].end
    } else {
        edges[last].start
    };
    (
        (point.x * SCALE).round() as i64,
        (point.y * SCALE).round() as i64,
    )
}

fn derived_refusal(entity: &DbEntity, reason: &str) -> CadError {
    CadError::Unsupported(format!(
        "area.object.vendor_outline: entity {} has a vendor-derived outline that cannot be \
         measured: {reason}",
        entity.id.0
    ))
}
