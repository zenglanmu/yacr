//! Screen → world ray → precise pick (spec F14 / F05).
//!
//! This module joins the two halves of picking that already exist elsewhere:
//!
//! * [`camera::Camera::screen_to_ray`] turns a logical screen pixel into a world
//!   [`Ray3`] for both projections (F13);
//! * [`cad_spatial::pick_closest`] runs the precise CPU hit test and returns the
//!   **closest** hit (F14).
//!
//! It adds the part that needs the database: walking model-space entities,
//! expanding `INSERT`s through their block definitions while accumulating the
//! [`InstancePath`], and deriving the world pick tolerance from the
//! [`TolerancePolicy`]. Two insert placements of the same block therefore
//! produce two distinct [`SelectionRef`]s and the hit carries the instance path
//! that separates them (audit F05).
//!
//! It never touches the GPU or a depth buffer: the pick is pure CPU geometry, so
//! it works on the WebGL2 path and in a host with no device (spec §8.5).
//!
//! What is deliberately *not* done here is recorded in `docs/picking-3d.md`:
//! sub-element (mesh-face) selection, GPU depth-test comparison and exact
//! analytic curve/line precision are open items; none of them is faked.

use crate::camera::{Camera, Projection};
use cad_db::{DrawingDatabase, MAX_INSTANCE_DEPTH};
use cad_domain::*;
use cad_spatial::{BackFacePolicy, PickItem, PickOptions, PickReport, SpatialIndex};

/// Derive the world-space pick tolerance from the tolerance policy.
///
/// A pick is a screen-space gesture: the policy expresses the interaction radius
/// in logical pixels ([`TolerancePolicy::interaction_logical_pixels`]) and the
/// projection converts it to world units.
///
/// * **Orthographic** — the scale is already world units per logical pixel.
/// * **Perspective** — the world size of a pixel grows with depth; a pixel at
///   the camera target distance spans `2·d·tan(fov/2) / height` world units.
///
/// The result is floored at [`TolerancePolicy::computation_world`] so a pick can
/// never be tighter than the numeric predicate tolerance, and is rejected when
/// non-finite (a degenerate viewport or projection).
pub fn pick_tolerance(
    policy: &TolerancePolicy,
    camera: &Camera,
    logical_size: [f64; 2],
) -> CadResult<f64> {
    camera.validate()?;
    if !logical_size[0].is_finite() || logical_size[0] <= 0.0 || !logical_size[1].is_finite() {
        return Err(CadError::InvalidInput(
            "pick viewport size must be finite and positive".into(),
        ));
    }
    let world_per_px = match camera.projection {
        Projection::Orthographic { scale } => scale,
        Projection::Perspective {
            vertical_fov_radians,
        } => {
            let height = logical_size[1].max(1.0);
            let depth = camera.distance().max(1e-9);
            2.0 * depth * (vertical_fov_radians * 0.5).tan() / height
        }
    };
    let tolerance = (policy.interaction_logical_pixels * world_per_px)
        .max(policy.computation_world)
        .max(f64::MIN_POSITIVE);
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(CadError::InvalidInput(
            "pick tolerance is not finite and positive".into(),
        ));
    }
    Ok(tolerance)
}

/// Collect the model-space pick items of a drawing.
///
/// `INSERT`s are expanded through their block definitions: the composed
/// transform is accumulated and the insert chain is recorded as the
/// [`InstancePath`] of every child, so two placements of one block yield two
/// distinct refs. Expansion is bounded by [`MAX_INSTANCE_DEPTH`] and cuts
/// cycles, exactly like [`DrawingDatabase::bounds`]; a truncated or cyclic
/// branch is skipped rather than producing a wrong hit.
pub fn drawing_pick_items(database: &DrawingDatabase, document: DocumentId) -> Vec<PickItem> {
    borrowed_pick_items(database, document)
        .into_iter()
        .map(|(source, geometry, transform)| PickItem {
            source,
            geometry: geometry.clone(),
            transform,
            geometry_source: match geometry {
                SemanticGeometry::Mesh(_) => GeometrySource::DirectMesh,
                _ => GeometrySource::Analytic,
            },
        })
        .collect()
}

fn borrowed_pick_items(
    database: &DrawingDatabase,
    document: DocumentId,
) -> Vec<(SelectionRef, &SemanticGeometry, Transform3)> {
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
    out: &mut Vec<(SelectionRef, &'a SemanticGeometry, Transform3)>,
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
                out,
            );
        }
        stack.pop();
        return;
    }
    out.push((
        SelectionRef {
            document,
            entity,
            instance: InstancePath(path.to_vec()),
            sub_element: None,
        },
        geometry,
        *transform,
    ));
}

/// Precise pick of the closest model-space geometry under a world ray.
///
/// Returns the closest hit with its distance and the list of geometries that
/// could not be tested (opaque payloads, unbounded branches). Degenerate rays
/// and tolerances are errors, never a silently empty report.
pub fn pick_ray(
    database: &DrawingDatabase,
    document: DocumentId,
    ray: &Ray3,
    options: &PickOptions,
) -> CadResult<PickReport> {
    let items = borrowed_pick_items(database, document);
    cad_spatial::pick_closest_borrowed(
        ray,
        items
            .iter()
            .map(|(source, geometry, transform)| (source, *geometry, transform)),
        options,
    )
}

/// Precise pick of the closest model-space geometry under a logical screen pixel.
///
/// Combines [`Camera::screen_to_ray`] with [`pick_ray`] and a tolerance derived
/// from `policy`. An invalid pixel or viewport (no ray) is [`CadError::InvalidInput`].
pub fn pick_at_screen(
    database: &DrawingDatabase,
    document: DocumentId,
    camera: &Camera,
    logical: [f64; 2],
    logical_size: [f64; 2],
    policy: &TolerancePolicy,
    back_faces: BackFacePolicy,
) -> CadResult<PickReport> {
    let ray = camera
        .screen_to_ray(logical, logical_size)
        .ok_or_else(|| CadError::InvalidInput("screen pixel does not produce a pick ray".into()))?;
    let tolerance = pick_tolerance(policy, camera, logical_size)?;
    let options = PickOptions::new(tolerance)?.with_back_faces(back_faces);
    pick_ray(database, document, &ray, &options)
}

/// Narrow a prepared item list to the candidates a broad-phase index reports.
///
/// The spatial index is an AABB broad phase only; this keeps the identity exact
/// (entity + instance path + sub-element) so a candidate of a different insert
/// instance is never substituted. The precise stage then runs over the result.
pub fn filter_by_index(
    index: &dyn SpatialIndex,
    ray: &Ray3,
    items: &[PickItem],
) -> CadResult<Vec<PickItem>> {
    let candidates = index.ray_candidates(ray)?;
    let matched: Vec<PickItem> = items
        .iter()
        .filter(|item| {
            candidates
                .iter()
                .any(|c| crate::selection::SelectionSet::same_object(c, &item.source))
        })
        .cloned()
        .collect();
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::SelectionSet;
    use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabaseBuilder, Layer};
    use cad_spatial::{GridSpatialIndex, SpatialEntry};

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3 { x, y, z }
    }

    fn entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        model_entity(id, geometry, draw_order)
    }

    fn model_entity(id: u128, geometry: SemanticGeometry, draw_order: i64) -> DbEntity {
        space_entity(id, geometry, draw_order, SpaceId::Model)
    }

    fn space_entity(
        id: u128,
        geometry: SemanticGeometry,
        draw_order: i64,
        space: SpaceId,
    ) -> DbEntity {
        DbEntity {
            object: DbObject {
                id: ObjectId(id),
                type_key: "AcDbEntity".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(id),
            layer: LayerId(0),
            space,
            geometry,
            draw_order,
        }
    }

    /// A drawing with two INSERT placements of the same block (one line each)
    /// at x = 10 and x = 20, plus a top-level line at x = 30.
    fn drawing() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(1));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(100)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(space_entity(
            100,
            SemanticGeometry::Line {
                start: p(0.0, -1.0, 0.0),
                end: p(0.0, 1.0, 0.0),
            },
            0,
            SpaceId::Block(BlockId(0)),
        ))
        .unwrap();
        b.insert_entity(entity(
            10,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(10.0, 0.0, 0.0)),
            },
            1,
        ))
        .unwrap();
        b.insert_entity(entity(
            20,
            SemanticGeometry::Insert {
                block: BlockId(0),
                transform: Transform3::translation(p(20.0, 0.0, 0.0)),
            },
            2,
        ))
        .unwrap();
        b.insert_entity(entity(
            30,
            SemanticGeometry::Line {
                start: p(30.0, -1.0, 0.0),
                end: p(30.0, 1.0, 0.0),
            },
            3,
        ))
        .unwrap();
        b.finish().unwrap()
    }

    fn camera_from(eye: Point3, target: Point3, projection: Projection) -> Camera {
        Camera {
            eye,
            target,
            up: p(0.0, 0.0, 1.0),
            projection,
        }
    }

    #[test]
    fn insert_expansion_carries_distinct_instance_paths() {
        let db = drawing();
        let items = drawing_pick_items(&db, DocumentId(1));
        // Two block children (one per insert) plus the top-level line.
        assert_eq!(items.len(), 3);
        let mut paths: Vec<Vec<EntityId>> =
            items.iter().map(|i| i.source.instance.0.clone()).collect();
        paths.sort();
        assert!(paths.contains(&vec![EntityId(10)]));
        assert!(paths.contains(&vec![EntityId(20)]));
        assert!(paths.contains(&Vec::new()));
    }

    #[test]
    fn two_insert_instances_are_separable_by_the_hit() {
        let db = drawing();
        let options = PickOptions::new(0.05).unwrap();
        // A ray along +x through the first placement hits the block child at
        // x = 10 and reports instance [10].
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let report = pick_ray(&db, DocumentId(1), &ray, &options).unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.entity, EntityId(100), "the block child entity");
        assert_eq!(hit.source.instance, InstancePath(vec![EntityId(10)]));
        // The other placeholder instance is at x = 20 — deeper than the hit at
        // x = 10, so the closest wins and the distance is honest.
        assert!((hit.distance - 10.0).abs() < 1e-9);
    }

    #[test]
    fn screen_pick_uses_camera_ray_and_returns_closest() {
        let db = drawing();
        // Orthographic top view: a pixel whose world column is x = 20 must hit
        // the second instance of the block child, not the first.
        let camera = Camera {
            eye: p(20.0, 0.0, 1000.0),
            target: p(20.0, 0.0, 0.0),
            up: p(0.0, 1.0, 0.0),
            projection: Projection::Orthographic { scale: 1.0 },
        };
        let size = [800.0, 600.0];
        let centre = [size[0] * 0.5, size[1] * 0.5];
        let report = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            centre,
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        let hit = report.hit.expect("a hit");
        assert_eq!(hit.source.instance, InstancePath(vec![EntityId(20)]));
        assert_eq!(hit.source.entity, EntityId(100));
    }

    #[test]
    fn screen_pick_misses_empty_space_without_a_wrong_hit() {
        let db = drawing();
        let camera = Camera::top_view_2d();
        let size = [800.0, 600.0];
        // A corner far from every line.
        let report = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            [2.0, 2.0],
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        assert!(report.hit.is_none());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn perspective_pick_hits_3d_geometry_along_the_view_ray() {
        // A vertical line at (5, 0, 0..10) is picked from an isometric-ish eye.
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(7));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(model_entity(
            1,
            SemanticGeometry::Line {
                start: p(5.0, 0.0, 0.0),
                end: p(5.0, 0.0, 10.0),
            },
            0,
        ))
        .unwrap();
        let db = b.finish().unwrap();
        let camera = camera_from(
            p(0.0, -20.0, 5.0),
            p(5.0, 0.0, 5.0),
            Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        );
        let size = [800.0, 600.0];
        // Project a point known to lie on the line, then pick that pixel.
        let on_line = p(5.0, 0.0, 5.0);
        let pixel = camera.world_to_screen(on_line, size).unwrap();
        let report = pick_at_screen(
            &db,
            DocumentId(7),
            &camera,
            pixel,
            size,
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        )
        .unwrap();
        let hit = report
            .hit
            .expect("the line is picked through its own pixel");
        assert_eq!(hit.source.entity, EntityId(1));
        assert!(hit.distance > 0.0);
    }

    #[test]
    fn pick_tolerance_scales_with_orthographic_zoom() {
        let policy = TolerancePolicy::default();
        let mut camera = Camera::top_view_2d();
        camera.projection = Projection::Orthographic { scale: 1.0 };
        let t1 = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        camera.projection = Projection::Orthographic { scale: 2.0 };
        let t2 = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        // Doubling world-units-per-pixel doubles the world tolerance.
        assert!((t2 - 2.0 * t1).abs() < 1e-9);
        assert!((t1 - policy.interaction_logical_pixels).abs() < 1e-9);
    }

    #[test]
    fn pick_tolerance_scales_inversely_with_perspective_height() {
        let policy = TolerancePolicy::default();
        let camera = camera_from(
            p(0.0, 0.0, 100.0),
            p(0.0, 0.0, 0.0),
            Projection::perspective(std::f64::consts::FRAC_PI_4).unwrap(),
        );
        let tall = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        let taller = pick_tolerance(&policy, &camera, [800.0, 1200.0]).unwrap();
        // More pixels over the same vertical FOV => narrower world-per-pixel.
        assert!(taller < tall);
        assert!((tall - 2.0 * taller).abs() < 1e-9);
    }

    #[test]
    fn pick_tolerance_never_drops_below_the_predicate_tolerance() {
        // A tiny interaction radius with a big predicate tolerance.
        let policy = TolerancePolicy {
            interaction_logical_pixels: 0.0,
            computation_world: 0.5,
            ..TolerancePolicy::default()
        };
        let camera = Camera::top_view_2d();
        let t = pick_tolerance(&policy, &camera, [800.0, 600.0]).unwrap();
        assert!((t - 0.5).abs() < 1e-12);
    }

    #[test]
    fn degenerate_viewport_has_no_pick_tolerance() {
        let policy = TolerancePolicy::default();
        let camera = Camera::top_view_2d();
        assert!(pick_tolerance(&policy, &camera, [0.0, 600.0]).is_err());
        assert!(pick_tolerance(&policy, &camera, [800.0, f64::NAN]).is_err());
    }

    #[test]
    fn invalid_screen_pixel_is_rejected_not_empty() {
        let db = drawing();
        let camera = Camera::top_view_2d();
        let result = pick_at_screen(
            &db,
            DocumentId(1),
            &camera,
            [f64::NAN, 0.0],
            [800.0, 600.0],
            &TolerancePolicy::default(),
            BackFacePolicy::Cull,
        );
        assert!(matches!(result, Err(CadError::InvalidInput(_))));
    }

    fn face(key: &str) -> SubElementId {
        SubElementId {
            source_key: key.into(),
            topology_revision: Revision(0),
        }
    }

    /// Two coplanar triangles at z = 2 (centres x = -1 and x = 1) with distinct
    /// stable face sources.
    fn two_face_mesh() -> Mesh {
        Mesh {
            vertices: vec![
                p(-2.0, -1.0, 2.0),
                p(-1.0, 1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(0.0, -1.0, 2.0),
                p(1.0, 1.0, 2.0),
                p(2.0, -1.0, 2.0),
            ],
            triangles: vec![[0, 1, 2], [3, 4, 5]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
        }
    }

    fn mesh_database() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(9));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_entity(entity(50, SemanticGeometry::Mesh(two_face_mesh()), 0))
            .unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn screen_pick_propagates_the_mesh_face_sub_element() {
        let db = mesh_database();
        let doc = DocumentId(9);
        let options = PickOptions::new(1e-6).unwrap();
        let a = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(-1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("face a is hit");
        let b = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("face b is hit");
        assert_eq!(a.source.entity, EntityId(50));
        assert_eq!(b.source.entity, EntityId(50));
        assert_eq!(
            a.source.sub_element.as_ref().map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        assert_eq!(
            b.source.sub_element.as_ref().map(|s| s.source_key.as_str()),
            Some("face-b")
        );
        assert_ne!(a.source, b.source, "two faces select differently");
        assert!(a.sub_element_reason.is_none());
    }

    #[test]
    fn mesh_without_a_face_source_is_unresolved_with_a_reason() {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(10));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        let mut mesh = two_face_mesh();
        mesh.face_sources = Vec::new();
        b.insert_entity(entity(51, SemanticGeometry::Mesh(mesh), 0))
            .unwrap();
        let db = b.finish().unwrap();
        let hit = pick_ray(
            &db,
            DocumentId(10),
            &Ray3 {
                origin: p(-1.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &PickOptions::new(1e-6).unwrap(),
        )
        .unwrap()
        .hit
        .expect("the mesh is still hit");
        // The entity is resolved; only the sub-element is not, and it says why.
        assert_eq!(hit.source.entity, EntityId(51));
        assert!(hit.source.sub_element.is_none());
        assert_eq!(
            hit.sub_element_reason,
            Some(cad_spatial::REASON_FACE_SOURCES_ABSENT)
        );
    }

    /// A block whose only child is a two-face mesh; two INSERT placements of the
    /// same block must still resolve the *same* face id on each instance.
    fn mesh_insert_database() -> DrawingDatabase {
        let mut b = DrawingDatabaseBuilder::new(DatabaseId(11));
        b.insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
        b.insert_block(BlockDefinition {
            id: BlockId(0),
            entities: vec![EntityId(200)],
            dynamic_visibility: None,
        })
        .unwrap();
        b.insert_entity(space_entity(
            200,
            SemanticGeometry::Mesh(two_face_mesh()),
            0,
            SpaceId::Block(BlockId(0)),
        ))
        .unwrap();
        for (id, x) in [(10u128, 10.0), (20, 20.0)] {
            b.insert_entity(entity(
                id,
                SemanticGeometry::Insert {
                    block: BlockId(0),
                    transform: Transform3::translation(p(x, 0.0, 0.0)),
                },
                id as i64,
            ))
            .unwrap();
        }
        b.finish().unwrap()
    }

    #[test]
    fn sub_element_is_stable_across_two_insert_instances() {
        let db = mesh_insert_database();
        let doc = DocumentId(11);
        let options = PickOptions::new(1e-6).unwrap();
        // The block's face A centre is local x = -1; instance 1 places it at
        // world x = 9, instance 2 at world x = 19.
        let first = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(9.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("first instance face is hit");
        let second = pick_ray(
            &db,
            doc,
            &Ray3 {
                origin: p(19.0, 0.0, 0.0),
                direction: p(0.0, 0.0, 1.0),
            },
            &options,
        )
        .unwrap()
        .hit
        .expect("second instance face is hit");

        assert_eq!(first.source.entity, EntityId(200));
        assert_eq!(first.source.instance, InstancePath(vec![EntityId(10)]));
        assert_eq!(second.source.entity, EntityId(200));
        assert_eq!(second.source.instance, InstancePath(vec![EntityId(20)]));
        // Same stable face id — the instance path is what separates them.
        assert_eq!(first.source.sub_element, second.source.sub_element);
        assert_eq!(
            first
                .source
                .sub_element
                .as_ref()
                .map(|s| s.source_key.as_str()),
            Some("face-a")
        );
        assert_ne!(first.source, second.source);
    }

    #[test]
    fn a_face_selection_differs_from_its_entity_and_from_an_edge() {
        let doc = DocumentId(9);
        let base = SelectionRef {
            document: doc,
            entity: EntityId(50),
            instance: InstancePath::default(),
            sub_element: None,
        };
        let face_ref = SelectionRef {
            sub_element: Some(face("face-a")),
            ..base.clone()
        };
        // A line/edge pick is whole-entity (sub_element stays None).
        let mut set = SelectionSet::new();
        assert!(set.insert(base.clone()));
        assert!(set.insert(face_ref.clone()));
        assert_eq!(set.len(), 2, "entity and one of its faces are distinct");
        assert_ne!(
            SelectionSet::identity_label(&base),
            SelectionSet::identity_label(&face_ref)
        );
    }

    #[test]
    fn filter_by_index_keeps_identity_exact() {
        let db = drawing();
        let items = drawing_pick_items(&db, DocumentId(1));
        let mut index = GridSpatialIndex::new();
        let entries: Vec<SpatialEntry> = items
            .iter()
            .map(|item| SpatialEntry {
                source: item.source.clone(),
                bounds: Bounds3 {
                    min: p(-1.0, -1.0, -1.0),
                    max: p(1.0, 1.0, 1.0),
                },
            })
            .collect();
        index.rebuild(&entries).unwrap();
        // A ray along +x passes through every item's box.
        let ray = Ray3 {
            origin: p(0.0, 0.0, 0.0),
            direction: p(1.0, 0.0, 0.0),
        };
        let filtered = filter_by_index(&index, &ray, &items).unwrap();
        assert_eq!(filtered.len(), items.len());
    }
}
