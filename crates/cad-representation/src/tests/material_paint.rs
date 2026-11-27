//! A painted entity draws in its material's flat colour.
//!
//! The assignment lives in the document material table, so the expansion looks
//! it up per entity and overrides the stored `ByLayer`/explicit look without an
//! entity write. These tests pin that rule, the transparency-to-alpha mapping
//! and the untouched unpainted path.

use super::*;
use cad_domain::material::MaterialDocument;

/// A model-space line on layer 0.
fn line(id: u128) -> DbEntity {
    line_entity(id, SpaceId::Model, p(0.0, 0.0), p(2.0, 0.0))
}

fn database(entities: Vec<DbEntity>, materials: MaterialDocument) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    for entity in entities {
        builder.insert_entity(entity).unwrap();
    }
    builder.set_materials(materials).unwrap();
    builder.finish().unwrap()
}

fn expand(db: &DrawingDatabase, id: u128) -> DisplayRepresentation {
    ProviderRegistry::with_default_provider()
        .build_expanded(db, db.entity(EntityId(id)).unwrap(), &context())
        .unwrap()
}

fn assert_fragment(fragment: &DisplayFragment, color: [u8; 3], alpha: f32) {
    for (channel, byte) in color.iter().enumerate() {
        let want = *byte as f32 / 255.0;
        assert!(
            (fragment.color[channel] - want).abs() < 1e-6,
            "channel {channel}: got {} want {want}",
            fragment.color[channel]
        );
    }
    assert!(
        (fragment.alpha - alpha).abs() < 1e-6,
        "alpha {} want {alpha}",
        fragment.alpha
    );
    assert!(
        !fragment.color_unresolved,
        "a paint is a concrete colour, never unresolved"
    );
}

fn first_fragment(representation: &DisplayRepresentation) -> &DisplayFragment {
    representation
        .fragments
        .first()
        .expect("the line expands to one fragment")
}

#[test]
fn a_painted_entity_draws_in_the_material_colour() {
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign(EntityId(7), red).unwrap();
    let db = database(vec![line(7)], materials);
    let representation = expand(&db, 7);
    assert_fragment(first_fragment(&representation), [178, 34, 34], 1.0);
}

#[test]
fn a_transparent_material_multiplies_the_resolved_alpha() {
    let mut materials = MaterialDocument::new();
    let glass = materials.ensure("Glass", [170, 220, 240], 0.6).unwrap();
    materials.assign(EntityId(7), glass).unwrap();
    let db = database(vec![line(7)], materials);
    let representation = expand(&db, 7);
    assert_fragment(first_fragment(&representation), [170, 220, 240], 0.4);
}

#[test]
fn a_paint_overrides_an_explicit_entity_colour_but_leaves_other_rows_alone() {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder.insert_entity(line(7)).unwrap();
    builder.insert_entity(line(8)).unwrap();
    builder
        .set_entity_render_attributes(
            EntityId(8),
            attrs_styled(
                cad_db::EntityColor::Explicit([10, 20, 30]),
                cad_db::EntityLineWeight::ByLayer,
            ),
        )
        .unwrap();
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign(EntityId(7), red).unwrap();
    builder.set_materials(materials).unwrap();
    let db = builder.finish().unwrap();

    assert_fragment(first_fragment(&expand(&db, 7)), [178, 34, 34], 1.0);
    // The unpainted row keeps its explicit colour.
    assert_fragment(first_fragment(&expand(&db, 8)), [10, 20, 30], 1.0);
}

/// Two triangles, each with its own persisted face identity.
fn two_face_mesh(id: u128) -> DbEntity {
    let face = |key: &str| SubElementId {
        source_key: key.into(),
        topology_revision: Revision(0),
    };
    entity(
        id,
        SemanticGeometry::Mesh(Mesh {
            vertices: vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0), p(1.0, 1.0)],
            triangles: vec![[0, 1, 2], [1, 3, 2]],
            normals: Vec::new(),
            face_sources: vec![Some(face("face-a")), Some(face("face-b"))],
            colors: Vec::new(),
            uvs: Vec::new(),
            texture: None,
            pbr: None,
        }),
    )
}

fn material_fragments(db: &DrawingDatabase, id: u128) -> Vec<DisplayFragment> {
    expand(db, id).fragments
}

fn fragment_with_face<'a>(fragments: &'a [DisplayFragment], face: &str) -> &'a DisplayFragment {
    fragments
        .iter()
        .find(|fragment| match &fragment.primitive {
            DisplayPrimitive::Mesh(mesh) | DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => mesh
                .face_sources
                .iter()
                .flatten()
                .any(|source| source.source_key == face),
            DisplayPrimitive::MeshWithStoredEdges { mesh, .. } => mesh
                .face_sources
                .iter()
                .flatten()
                .any(|source| source.source_key == face),
            _ => false,
        })
        .unwrap_or_else(|| panic!("no fragment carries face {face}"))
}

#[test]
fn a_face_paint_splits_the_mesh_and_only_that_face_changes_colour() {
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials.assign_face(EntityId(7), "face-b", red).unwrap();
    let db = database(vec![two_face_mesh(7)], materials);
    let fragments = material_fragments(&db, 7);
    assert_eq!(fragments.len(), 2, "one fragment per material group");
    let red_fragment = fragment_with_face(&fragments, "face-b");
    assert_fragment(red_fragment, [178, 34, 34], 1.0);
    match &red_fragment.primitive {
        DisplayPrimitive::Mesh(mesh) => {
            assert_eq!(mesh.triangles.len(), 1, "only the painted triangle");
            assert_eq!(
                mesh.face_sources[0]
                    .as_ref()
                    .map(|sub| sub.source_key.as_str()),
                Some("face-b")
            );
        }
        _ => panic!("expected a mesh fragment"),
    }
    let other = fragment_with_face(&fragments, "face-a");
    assert!(!(other.color[0] > 0.6 && other.color[1] < 0.2), "not red");
}

#[test]
fn a_transparent_face_material_multiplies_the_entity_alpha() {
    let mut materials = MaterialDocument::new();
    let glass = materials.ensure("Glass", [170, 220, 240], 0.6).unwrap();
    materials.assign_face(EntityId(7), "face-a", glass).unwrap();
    let db = database(vec![two_face_mesh(7)], materials);
    let fragments = material_fragments(&db, 7);
    let glass_fragment = fragment_with_face(&fragments, "face-a");
    assert_fragment(glass_fragment, [170, 220, 240], 0.4);
}

#[test]
fn a_face_paint_whose_face_disappeared_is_reported_not_remapped() {
    let mut materials = MaterialDocument::new();
    let red = materials.ensure("Red", [178, 34, 34], 0.0).unwrap();
    materials
        .assign_face(EntityId(7), "face-gone", red)
        .unwrap();
    let db = database(vec![two_face_mesh(7)], materials);
    let representation = expand(&db, 7);
    assert_eq!(
        representation.fragments.len(),
        1,
        "no triangle is remapped to the missing key"
    );
    assert!(representation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "representation.material_face_inert"));
}
