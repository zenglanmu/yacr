//! The vendor-aware half of the model-space pick catalogue (ADR-0028 D2).
//!
//! `cad-db` remains the only authority: a TCH entity is stored as
//! [`SemanticGeometry::Opaque`] and is never rewritten. When a plugin pass has
//! settled a [`VendorRepresentationStore`] for the source handle, this module
//! offers the entity's **derived** primitives as analytic pick geometry, still
//! identified by the committed database entity ([`SelectionRef`] with the real
//! `EntityId` and instance path).
//!
//! The split lives here — not in `picking::mod` — so the catalogue expansion,
//! its `Cow` view and the primitive mapping stay together and the module stays
//! within the repository's file-size limit.

use crate::vendor::{VendorPrimitive, VendorRepresentationStore};
use cad_db::{DrawingDatabase, MAX_INSTANCE_DEPTH};
use cad_domain::*;
use cad_spatial::PickItem;
use std::borrow::Cow;

/// The vendor-aware catalogue: the same expansion as
/// [`super::drawing_pick_items`], except that an entity whose geometry is
/// exactly [`SemanticGeometry::Opaque`] contributes one synthesized analytic
/// stand-in when the settled snapshot has primitives for its `source_handle`.
///
/// The item's identity is unchanged — `source` is the real
/// `SelectionRef { document, entity, instance }` of the committed database
/// entity — and `geometry_source` is [`GeometrySource::ProxyCache`]. The
/// primitives are in the source entity's local coordinates, so the item keeps
/// the accumulated `INSERT` transform and is **not** pre-transformed.
///
/// The database is never written: this derives pick geometry from a display
/// snapshot, exactly like the render assembly does.
pub fn drawing_pick_items_with_vendor(
    database: &DrawingDatabase,
    document: DocumentId,
    vendor: Option<&VendorRepresentationStore>,
) -> Vec<PickItem> {
    borrowed_pick_items(database, document, vendor)
        .into_iter()
        .map(|item| PickItem {
            source: item.source,
            geometry: item.geometry.into_owned(),
            transform: item.transform,
            geometry_source: item.geometry_source,
        })
        .collect()
}

/// One expanded pick item before it is materialized for the precise stage.
///
/// The geometry is borrowed from the database for ordinary entities and owned
/// for a vendor-backed opaque entity, whose analytic stand-in is synthesized on
/// the fly. Keeping the distinction in a `Cow` means the non-vendor hot path
/// never deep-copies a drawing just to run a read-only query.
pub(super) struct PickItemView<'a> {
    pub(super) source: SelectionRef,
    pub(super) geometry: Cow<'a, SemanticGeometry>,
    pub(super) transform: Transform3,
    pub(super) geometry_source: GeometrySource,
}

pub(super) fn borrowed_pick_items<'a>(
    database: &'a DrawingDatabase,
    document: DocumentId,
    vendor: Option<&VendorRepresentationStore>,
) -> Vec<PickItemView<'a>> {
    let mut items = Vec::new();
    let mut stack: Vec<BlockId> = Vec::new();
    for entity in database.model_space() {
        collect_items(
            database,
            entity.id,
            &entity.geometry,
            document,
            &Transform3::identity(),
            &[],
            0,
            &mut stack,
            vendor,
            &mut items,
        );
    }
    items
}

#[allow(clippy::too_many_arguments)]
fn collect_items<'a>(
    database: &'a DrawingDatabase,
    entity: EntityId,
    geometry: &'a SemanticGeometry,
    document: DocumentId,
    transform: &Transform3,
    path: &[EntityId],
    depth: usize,
    stack: &mut Vec<BlockId>,
    vendor: Option<&VendorRepresentationStore>,
    out: &mut Vec<PickItemView<'a>>,
) {
    if let SemanticGeometry::Insert {
        block,
        transform: insert,
    } = geometry
    {
        if depth >= MAX_INSTANCE_DEPTH || stack.contains(block) {
            return;
        }
        let composed = transform.matrix_mul(insert);
        let mut child_path = path.to_vec();
        child_path.push(entity);
        stack.push(*block);
        for child in database.block_entities(*block) {
            collect_items(
                database,
                child.id,
                &child.geometry,
                document,
                &composed,
                &child_path,
                depth + 1,
                stack,
                vendor,
                out,
            );
        }
        stack.pop();
        return;
    }
    // Only an exactly-opaque entity can be replaced by vendor geometry; every
    // other family keeps the borrowed database geometry (and its source tag).
    let synthesized = if matches!(geometry, SemanticGeometry::Opaque { .. }) {
        database
            .entity(entity)
            .and_then(|row| vendor.and_then(|store| vendor_pick_geometry(row, store)))
    } else {
        None
    };
    let (geometry, geometry_source) = match synthesized {
        Some(synthesized) => (Cow::Owned(synthesized), GeometrySource::ProxyCache),
        None => (
            Cow::Borrowed(geometry),
            match geometry {
                SemanticGeometry::Mesh(_) => GeometrySource::DirectMesh,
                _ => GeometrySource::Analytic,
            },
        ),
    };
    out.push(PickItemView {
        source: SelectionRef {
            document,
            entity,
            instance: InstancePath(path.to_vec()),
            sub_element: None,
        },
        geometry,
        transform: *transform,
        geometry_source,
    });
}

/// The synthesized analytic pick geometry for a vendor-backed opaque entity.
///
/// `None` unless the entity is exactly `Opaque`, carries a `source_handle`, the
/// settled store has primitives for that handle, and the primitives yield at
/// least one pickable child. An entity whose snapshot holds only `Block`
/// primitives therefore stays unpickable and keeps its original `Opaque` item
/// (ADR-0028 §4: a documented limitation, never a fake hit).
///
/// Also used by `SELECTALL` / box-selection preflight so "vendor-backed" means
/// exactly the same thing on every path.
pub(crate) fn vendor_pick_geometry(
    entity: &cad_db::DbEntity,
    store: &VendorRepresentationStore,
) -> Option<SemanticGeometry> {
    if !matches!(entity.geometry, SemanticGeometry::Opaque { .. }) {
        return None;
    }
    let handle = entity.object.source_handle.as_deref()?;
    let primitives = store.for_source_handle(handle)?;
    if !vendor_primitives_have_pick_geometry(primitives) {
        return None;
    }
    Some(SemanticGeometry::Compound(vendor_pick_children(primitives)))
}

/// Whether this entity is an `Opaque` payload with a non-empty vendor stand-in.
///
/// This is the **allocation-free** predicate used by `SELECTALL` / box preflight:
/// it applies the same rule as [`vendor_pick_geometry`] (including the
/// block-only refusal) without cloning the primitive points just to discard the
/// synthesized compound.
pub(crate) fn vendor_backed_entity(
    entity: &cad_db::DbEntity,
    store: &VendorRepresentationStore,
) -> bool {
    if !matches!(entity.geometry, SemanticGeometry::Opaque { .. }) {
        return false;
    }
    let Some(handle) = entity.object.source_handle.as_deref() else {
        return false;
    };
    store
        .for_source_handle(handle)
        .is_some_and(vendor_primitives_have_pick_geometry)
}

/// The mapping rule without building anything: at least one primitive yields a
/// pickable child (`Block` never does; degenerate line/segment lists never do).
pub(crate) fn vendor_primitives_have_pick_geometry(primitives: &[VendorPrimitive]) -> bool {
    primitives.iter().any(|primitive| match primitive {
        VendorPrimitive::Lines(points) => points.len() >= 2,
        VendorPrimitive::Segments(points) => points.len() >= 2 && points.len().is_multiple_of(2),
        VendorPrimitive::Arc { .. } | VendorPrimitive::Circle { .. } | VendorPrimitive::Text(_) => {
            true
        }
        VendorPrimitive::Block { .. } => false,
    })
}

/// Map one snapshot's display primitives to analytic pick geometry.
///
/// * `Lines` → one open `Polyline`;
/// * `Segments` → one `Line` per consecutive pair (an odd trailing point is a
///   malformed primitive and contributes nothing);
/// * `Arc`/`Circle` → the matching analytic curve on the XY plane;
/// * `Text` → its anchor `Point` only (glyph outlines are **not** fabricated);
/// * `Block` → skipped: expanding an imported definition on the pick path is a
///   documented limitation (ADR-0028 D2).
fn vendor_pick_children(primitives: &[VendorPrimitive]) -> Vec<SemanticGeometry> {
    let mut children = Vec::new();
    for primitive in primitives {
        match primitive {
            VendorPrimitive::Lines(points) if points.len() >= 2 => {
                children.push(SemanticGeometry::Polyline {
                    points: points.clone(),
                    bulges: Vec::new(),
                    closed: false,
                    width: 0.0,
                });
            }
            VendorPrimitive::Segments(points)
                if points.len() >= 2 && points.len().is_multiple_of(2) =>
            {
                for pair in points.as_chunks::<2>().0 {
                    children.push(SemanticGeometry::Line {
                        start: pair[0],
                        end: pair[1],
                    });
                }
            }
            VendorPrimitive::Arc {
                center,
                radius,
                start,
                sweep,
            } => children.push(SemanticGeometry::Arc {
                center: *center,
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: *radius,
                start: *start,
                sweep: *sweep,
            }),
            VendorPrimitive::Circle { center, radius } => children.push(SemanticGeometry::Circle {
                center: *center,
                normal: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 1.0,
                },
                radius: *radius,
            }),
            VendorPrimitive::Text(text) => {
                children.push(SemanticGeometry::Point(text.origin));
            }
            VendorPrimitive::Block { .. } => {}
            // A `Lines` primitive with fewer than two points, or an odd-length
            // `Segments` list, has no pickable edge at all.
            VendorPrimitive::Lines(_) | VendorPrimitive::Segments(_) => {}
        }
    }
    children
}
