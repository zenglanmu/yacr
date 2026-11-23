//! IFC contract tests: both schemas, placement, opening subtraction, units,
//! refusal channels, limits, cancellation, mesh invariants and detection.

use std::cell::Cell;
use std::sync::Arc;

use super::*;
use cad_domain::{
    CadError, Completeness, DatabaseId, DocumentId, EntityCapability, Mesh, SemanticGeometry,
    SupportStatus,
};
use cad_import_core::{
    detect_format, DrawingFormat, ImportLimits, ImportPhase, IFC_SCHEMA_REFUSED,
};
use cad_connector_step::StepImporter;

fn request_bytes(bytes: Vec<u8>) -> ImportRequest {
    ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
        xref: None,
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ifc")
        .join(name);
    std::fs::read(path).expect("fixture is committed")
}

fn import_fixture(name: &str) -> cad_import_core::ImportedDrawing {
    IfcImporter::new()
        .import(&request_bytes(fixture(name)), &|| false)
        .unwrap_or_else(|error| panic!("{name} must import: {error:?}"))
}

fn meshes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&Mesh> {
    drawing
        .database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .inspect(|mesh| assert_mesh_invariants(mesh))
        .collect()
}

/// The invariants `cad-db::validate_geometry` and every consumer rely on.
fn assert_mesh_invariants(mesh: &Mesh) {
    assert!(
        mesh.normals.is_empty() || mesh.normals.len() == mesh.vertices.len(),
        "mesh normals must be empty or one per vertex"
    );
    assert!(
        mesh.colors.is_empty() || mesh.colors.len() == mesh.vertices.len(),
        "mesh colours must be empty or one per vertex"
    );
    assert!(
        mesh.uvs.is_empty() || mesh.uvs.len() == mesh.vertices.len(),
        "mesh UVs must be empty or one pair per vertex"
    );
    if mesh.texture.is_some() {
        assert!(!mesh.uvs.is_empty(), "a textured mesh must carry UVs");
    }
    assert!(!mesh.vertices.is_empty(), "a stored mesh has vertices");
    assert!(!mesh.triangles.is_empty(), "a stored mesh has triangles");
    for vertex in &mesh.vertices {
        assert!(
            vertex.x.is_finite() && vertex.y.is_finite() && vertex.z.is_finite(),
            "mesh vertex is non-finite"
        );
    }
    for normal in &mesh.normals {
        assert!(
            normal.x.is_finite() && normal.y.is_finite() && normal.z.is_finite(),
            "mesh normal is non-finite"
        );
    }
    let count = mesh.vertices.len() as u32;
    for triangle in &mesh.triangles {
        assert!(
            triangle.iter().all(|index| *index < count),
            "mesh triangle references an unknown vertex"
        );
        assert!(
            triangle[0] != triangle[1] && triangle[1] != triangle[2] && triangle[0] != triangle[2],
            "mesh triangle is degenerate"
        );
    }
}

fn diagnostic_codes(drawing: &cad_import_core::ImportedDrawing) -> Vec<&str> {
    drawing
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect()
}

fn triangle_total(meshes: &[&Mesh]) -> usize {
    meshes.iter().map(|mesh| mesh.triangles.len()).sum()
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: [f64; 3],
    max: [f64; 3],
}

fn bounds(mesh: &Mesh) -> Bounds {
    let mut bounds = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for vertex in &mesh.vertices {
        for (axis, value) in [vertex.x, vertex.y, vertex.z].into_iter().enumerate() {
            bounds.min[axis] = bounds.min[axis].min(value);
            bounds.max[axis] = bounds.max[axis].max(value);
        }
    }
    bounds
}

fn extent(bounds: &Bounds, axis: usize) -> f64 {
    bounds.max[axis] - bounds.min[axis]
}

#[test]
fn ifc4_wall_imports_one_mesh_in_metres() {
    let drawing = import_fixture("synthetic-wall.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "one product becomes one DbEntity");
    // A 4000 × 200 × 3000 mm box extrusion is 12 triangles.
    assert_eq!(triangle_total(&meshes), 12);
    // The file declares millimetres; the router scales to metres, so a
    // 4000 × 200 × 3000 mm wall is 4 × 0.2 × 3 m in the database.
    let bounds = bounds(meshes[0]);
    assert!((extent(&bounds, 0) - 4.0).abs() < 1e-3, "{bounds:?}");
    assert!((extent(&bounds, 1) - 0.2).abs() < 1e-3, "{bounds:?}");
    assert!((extent(&bounds, 2) - 3.0).abs() < 1e-3, "{bounds:?}");
    // Stored metres, displayed in the authored millimetres.
    assert_eq!(drawing.units.source, Unit::Meter);
    assert_eq!(drawing.units.display, Unit::Millimeter);
    assert_eq!(drawing.units.display_per_source, Some(1000.0));
    assert_eq!(drawing.units.to_display(4.0), Some(4000.0));
    assert_eq!(
        drawing.report.completeness,
        Completeness::Complete,
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_CANONICALISED));
    let capability: &EntityCapability = &drawing.report.capabilities[0];
    assert_eq!(capability.type_key, IFC_MESH_TYPE_KEY);
    assert_eq!(capability.read, SupportStatus::Verified);
    assert_eq!(capability.render, SupportStatus::Verified);
    assert_eq!(capability.pick, SupportStatus::Verified);
    assert_eq!(capability.measure, SupportStatus::Unsupported);
    assert!(
        drawing.vendor_records.is_empty(),
        "IFC has no vendor records"
    );
}

#[test]
fn ifc2x3_two_walls_keep_distinct_placed_bounds() {
    let drawing = import_fixture("synthetic-two-walls-ifc2x3.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 2, "two products become two DbEntities");
    assert_eq!(
        triangle_total(&meshes),
        24,
        "each wall is a 12-triangle box"
    );
    let mut ordered: Vec<Bounds> = meshes.iter().map(|mesh| bounds(mesh)).collect();
    ordered.sort_by(|left, right| left.min[0].total_cmp(&right.min[0]));
    // Wall A: x 0..4000 mm, wall B placed at x=6000 mm. If placement were not
    // baked in, both would sit at the origin with identical bounds.
    assert!((ordered[0].min[0] - 0.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[0].max[0] - 4.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[1].min[0] - 6.0).abs() < 1e-3, "{ordered:?}");
    assert!((ordered[1].max[0] - 10.0).abs() < 1e-3, "{ordered:?}");
    assert!(
        ordered[0].max[0] <= ordered[1].min[0],
        "placed walls must not overlap: {ordered:?}"
    );
    assert_eq!(drawing.units.source, Unit::Meter);
}

#[test]
fn opening_subtraction_adds_triangles() {
    let uncut = import_fixture("synthetic-wall.ifc");
    let cut = import_fixture("synthetic-wall-opening.ifc");
    let uncut_meshes = meshes(&uncut);
    let cut_meshes = meshes(&cut);
    assert_eq!(uncut_meshes.len(), 1);
    assert_eq!(
        cut_meshes.len(),
        1,
        "the opening is a cutter, not a visible product"
    );
    let uncut_triangles = triangle_total(&uncut_meshes);
    let cut_triangles = triangle_total(&cut_meshes);
    assert_eq!(uncut_triangles, 12, "the uncut wall is a 12-triangle box");
    assert_eq!(
        cut_triangles, 32,
        "the opening cuts the box and adds reveal faces (12 -> 32)"
    );
    assert!(
        cut_triangles > uncut_triangles,
        "an opening must add reveal geometry: {uncut_triangles} -> {cut_triangles}"
    );
    // The cut must not move the wall: the outer bounds stay the same.
    let before = bounds(uncut_meshes[0]);
    let after = bounds(cut_meshes[0]);
    for axis in 0..3 {
        assert!(
            (before.min[axis] - after.min[axis]).abs() < 1e-3
                && (before.max[axis] - after.max[axis]).abs() < 1e-3,
            "opening moved the wall on axis {axis}: {before:?} -> {after:?}"
        );
    }
    assert!(
        !diagnostic_codes(&cut).contains(&DIAG_CSG_FAILURE),
        "the fixture's opening must cut cleanly: {:?}",
        cut.report.diagnostics
    );
}

#[test]
fn mesh_invariants_hold_for_every_imported_fixture() {
    for name in [
        "synthetic-wall.ifc",
        "synthetic-wall-opening.ifc",
        "synthetic-two-walls-ifc2x3.ifc",
    ] {
        let drawing = import_fixture(name);
        let meshes = meshes(&drawing);
        assert!(!meshes.is_empty(), "{name} imports at least one mesh");
        for mesh in meshes {
            assert_mesh_invariants(mesh);
        }
    }
}

#[test]
fn an_unresolved_length_unit_is_drawing_units_and_partial() {
    let drawing = import_fixture("synthetic-wall-no-units.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1);
    assert_eq!(drawing.units.source, Unit::DrawingUnits);
    assert_eq!(drawing.units.display, Unit::DrawingUnits);
    assert_eq!(
        drawing.units.display_per_source, None,
        "an unknown unit must not invent a scale"
    );
    // No declared unit means the router's scale stays 1.0: the raw file numbers
    // (4000, not 4 metres) are what is stored.
    assert!((extent(&bounds(meshes[0]), 0) - 4000.0).abs() < 1e-3);
    assert!(diagnostic_codes(&drawing).contains(&DIAG_UNITS_UNKNOWN));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn every_element_refused_is_an_error_not_an_empty_success() {
    let error = IfcImporter::new()
        .import(&request_bytes(fixture("synthetic-axis-only.ifc")), &|| {
            false
        })
        .err()
        .expect("an axis-only wall has no drawable mesh");
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(ALL_ELEMENTS_FAILED),
            "stable code missing: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn over_limit_file_size_is_invalid_input() {
    let mut request = request_bytes(fixture("synthetic-wall.ifc"));
    request.limits.max_file_bytes = 128;
    let error = IfcImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("file exceeds the byte limit");
    assert!(matches!(error, CadError::InvalidInput(_)), "{error:?}");
    assert!(error.to_string().contains("byte import limit"));
}

#[test]
fn over_limit_triangle_count_is_invalid_input() {
    let mut request = request_bytes(fixture("synthetic-wall.ifc"));
    request.limits.max_mesh_faces = Some(1);
    let error = IfcImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the wall exceeds one triangle");
    match error {
        CadError::InvalidInput(message) => assert!(
            message.contains("triangle") && message.contains('1'),
            "counts missing: {message}"
        ),
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn garbage_bytes_are_corrupt_data() {
    let error = IfcImporter::new()
        .import(&request_bytes(b"hello world".to_vec()), &|| false)
        .err()
        .expect("not an IFC file");
    match error {
        CadError::CorruptData(message) => assert!(
            message.contains(NO_GEOMETRY_PRODUCTS),
            "stable code missing: {message}"
        ),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn truncated_ifc_is_corrupt_data() {
    let truncated = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
                      #1=IFCWALL('2O2Fr$t4X7Zf8NOew3FL"
        .to_vec();
    let error = IfcImporter::new()
        .import(&request_bytes(truncated), &|| false)
        .err()
        .expect("a truncated DATA section holds no product");
    match error {
        CadError::CorruptData(message) => assert!(
            message.contains(NO_GEOMETRY_PRODUCTS),
            "stable code missing: {message}"
        ),
        other => panic!("expected CorruptData, got {other:?}"),
    }
}

#[test]
fn cancellation_is_honoured() {
    let calls = Cell::new(0usize);
    let cancelled = || {
        calls.set(calls.get() + 1);
        calls.get() > 1
    };
    let error = IfcImporter::new()
        .import(&request_bytes(fixture("synthetic-wall.ifc")), &cancelled)
        .err()
        .expect("cancellation stops the import");
    assert!(matches!(error, CadError::Cancelled), "{error:?}");
}

#[test]
fn ifc_detection_differs_from_step_by_file_schema() {
    let ifc = fixture("synthetic-wall.ifc");
    assert_eq!(detect_format(&ifc), DrawingFormat::Ifc);
    assert!(ifc_file_schema(&ifc).is_some());
    let step = b"ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\n";
    assert_eq!(detect_format(step), DrawingFormat::Step);
    assert!(ifc_file_schema(step).is_none());
}

#[test]
fn ifc_never_reaches_the_step_importer() {
    let request = request_bytes(fixture("synthetic-wall.ifc"));
    let error = StepImporter::new()
        .import(&request, &|| false)
        .err()
        .expect("the STEP adapter must refuse IFC bytes");
    match error {
        CadError::Unsupported(message) => assert!(
            message.contains(IFC_SCHEMA_REFUSED),
            "stable code missing: {message}"
        ),
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn real_ifc_from_env_is_imported() {
    // Opt-in real-file check (`YACR_TEST_IFC`), same convention as
    // `YACR_TEST_DWG`. A skip is not a run and not evidence.
    let Ok(path) = std::env::var("YACR_TEST_IFC") else {
        return;
    };
    let bytes = std::fs::read(&path).expect("YACR_TEST_IFC must name a readable file");
    let drawing = IfcImporter::new()
        .import(&request_bytes(bytes), &|| false)
        .unwrap_or_else(|error| panic!("{path} must import: {error:?}"));
    assert!(
        meshes(&drawing)
            .iter()
            .any(|mesh| !mesh.triangles.is_empty()),
        "{path} produced no drawable mesh"
    );
}

#[test]
fn an_opening_without_a_host_is_reported_not_drawn() {
    let drawing = import_fixture("synthetic-orphan-opening.ifc");
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "the wall is the only visible product");
    assert_eq!(triangle_total(&meshes), 12, "the wall stays uncut");
    assert!(diagnostic_codes(&drawing).contains(&DIAG_OPENING_WITHOUT_HOST));
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn progress_reaches_every_phase_with_real_counts() {
    let phases = std::sync::Mutex::new(Vec::<ImportPhase>::new());
    let sink = |progress: ImportProgress| phases.lock().unwrap().push(progress.phase);
    IfcImporter::new()
        .import_with_progress(
            &request_bytes(fixture("synthetic-two-walls-ifc2x3.ifc")),
            &|| false,
            &sink,
        )
        .expect("two-wall IFC imports");
    let phases = phases.into_inner().unwrap();
    for expected in [
        ImportPhase::Reading,
        ImportPhase::Parsing,
        ImportPhase::Entities,
        ImportPhase::Finishing,
    ] {
        assert!(phases.contains(&expected), "missing phase {expected:?}");
    }
}

// ---------------------------------------------------------------- textures ---

/// A one-quad `IfcTriangulatedFaceSet` (metres, 1 m used band) with a single
/// `IfcIndexedTriangleTextureMap` over it.
///
/// `texture` is the `#20` record, so the same document can carry an embedded
/// `IfcPixelTexture` or an `IfcImageTexture` pointing outside the model.
fn textured_tessellation(texture: &str) -> Vec<u8> {
    textured_tessellation_with(texture, "")
}

/// [`textured_tessellation`] plus extra DATA records (`extra` is spliced in
/// before `ENDSEC`), used to give the panel an opening.
fn textured_tessellation_with(texture: &str, extra: &str) -> Vec<u8> {
    format!(
        "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [CoordinationView]'),'2;1');
FILE_NAME('synthetic-textured-tessellation.ifc','2026-11-20T00:00:00',('yacr'),('yacr'),\
 'yacr synthetic fixture','yacr','');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IFCPROJECT('2O2Fr$t4X7Zf8NOew3FL00',$,'Textured project',$,$,$,$,(#2),#9);
#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#3,$);
#3=IFCAXIS2PLACEMENT3D(#4,#5,#6);
#4=IFCCARTESIANPOINT((0.,0.,0.));
#5=IFCDIRECTION((0.,0.,1.));
#6=IFCDIRECTION((1.,0.,0.));
#7=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);
#8=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);
#9=IFCUNITASSIGNMENT((#7,#8));
#10=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(1.,0.,0.),(1.,1.,0.),(0.,1.,0.)),$);
#11=IFCTRIANGULATEDFACESET(#10,$,.T.,((1,2,3),(1,3,4)),$);
#12=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#11));
#13=IFCPRODUCTDEFINITIONSHAPE($,$,(#12));
#14=IFCLOCALPLACEMENT($,#15);
#15=IFCAXIS2PLACEMENT3D(#16,$,$);
#16=IFCCARTESIANPOINT((0.,0.,0.));
#17=IFCBUILDINGELEMENTPROXY('2O2Fr$t4X7Zf8NOew3FL01',$,'Panel',$,$,#14,#13,$,$);
#20={texture};
#21=IFCTEXTUREVERTEXLIST(((0.,0.),(1.,0.),(1.,1.),(0.,1.)));
#22=IFCINDEXEDTRIANGLETEXTUREMAP((#20),#11,#21,((1,2,3),(1,3,4)));
{extra}ENDSEC;
END-ISO-10303-21;
"
    )
    .into_bytes()
}

/// Two RGB pixels: red then green.
const PIXEL_TEXTURE_2X1: &str =
    "IFCPIXELTEXTURE(.T.,.T.,'MODULATE',$,$,2,1,3,(\"0FF0000\",\"000FF00\"))";

/// `IfcImageTexture` names a sibling file; its bytes are not in the model.
const EXTERNAL_IMAGE_TEXTURE: &str = "IFCIMAGETEXTURE(.T.,.T.,$,$,$,'panel.png')";

fn import_textured(texture: &str) -> cad_import_core::ImportedDrawing {
    IfcImporter::new()
        .import(&request_bytes(textured_tessellation(texture)), &|| false)
        .unwrap_or_else(|error| panic!("the textured fixture must import: {error:?}"))
}

/// `(width, height, pixels)` of a PNG, decoded with the same crate the adapter
/// encodes with, so the test proves the container really holds the image.
fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let cursor = std::io::Cursor::new(bytes);
    let mut reader = png::Decoder::new(cursor).read_info().expect("PNG header");
    let size = reader.output_buffer_size().expect("bounded PNG");
    let mut pixels = vec![0u8; size];
    let info = reader.next_frame(&mut pixels).expect("PNG pixels");
    pixels.truncate(info.line_size * info.height as usize);
    (info.width, info.height, pixels)
}

#[test]
fn a_textured_tessellation_imports_uvs_and_an_embedded_png() {
    let drawing = import_textured(PIXEL_TEXTURE_2X1);
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "one textured face set is one entity");
    let mesh = meshes[0];
    // ifc-lite flat-shades a triangulated face set and then welds the positions
    // back together (indices only), so the four authored points stay four
    // vertices and the UVs stay aligned with them.
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.triangles.len(), 2);
    assert_eq!(
        mesh.uvs.len(),
        mesh.vertices.len(),
        "one UV pair per vertex"
    );
    // The fixture authors the UVs as (0,0) (1,0) (1,1) (0,1) for the points
    // (0,0) (1,0) (1,1) (0,1), and ifc-lite flips V to a top-left origin, so
    // every verified vertex must read `[x, 1 - y]`.
    for (vertex, uv) in mesh.vertices.iter().zip(&mesh.uvs) {
        assert_eq!(
            *uv,
            [vertex.x, 1.0 - vertex.y],
            "UV does not match the authored texture vertex for {vertex:?}"
        );
    }

    let key = mesh
        .texture
        .as_deref()
        .expect("the pixel texture is registered");
    assert!(
        key.strip_suffix(".png")
            .is_some_and(|stem| stem.len() == 32),
        "the key is content-addressed: {key}"
    );
    let bytes = drawing
        .database
        .texture(key)
        .expect("the document holds the image bytes");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "the asset is a real PNG");
    let (width, height, pixels) = decode_png(bytes);
    assert_eq!((width, height), (2, 1));
    assert_eq!(
        pixels,
        vec![255, 0, 0, 255, 0, 255, 0, 255],
        "the two authored RGB pixels, expanded to RGBA8"
    );
    assert_eq!(
        drawing.report.completeness,
        Completeness::Complete,
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
}

#[test]
fn an_external_image_texture_is_reported_and_never_read() {
    let drawing = import_textured(EXTERNAL_IMAGE_TEXTURE);
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "the geometry is kept");
    let mesh = meshes[0];
    assert_eq!(
        mesh.uvs.len(),
        mesh.vertices.len(),
        "the parameterisation is independent of the image"
    );
    assert!(
        mesh.texture.is_none(),
        "an external file is never read into the document"
    );
    assert_eq!(
        drawing.database.textures().count(),
        0,
        "no image bytes enter the document"
    );
    assert!(
        diagnostic_codes(&drawing).contains(&DIAG_EXTERNAL_TEXTURE_UNAVAILABLE),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}

#[test]
fn the_ifc_png_encoder_round_trips_into_register_texture() {
    // The encoder is the adapter's own bridge from IFC's decoded RGBA8 to a
    // document asset; this pins the exact byte plumbing `register_texture`
    // needs (PNG signature + decodable pixels) without a document fixture.
    let rgba = vec![10u8, 20, 30, 255, 40, 50, 60, 255];
    let png = texture::encode_rgba_png(&rgba, 2, 1).expect("2x1 RGBA encodes");
    let (width, height, pixels) = decode_png(&png);
    assert_eq!((width, height), (2, 1));
    assert_eq!(pixels, rgba);

    // Deterministic: the key is the hash of these bytes, so the same pixels must
    // produce the same PNG on every call.
    assert_eq!(
        png,
        texture::encode_rgba_png(&rgba, 2, 1).expect("encodes again")
    );

    // A short/long buffer is refused instead of being padded or truncated.
    for (bytes, width, height) in [
        (rgba.as_slice(), 3u32, 1u32),
        (rgba.as_slice(), 2, 2),
        (rgba.as_slice(), 0, 1),
    ] {
        assert!(
            texture::encode_rgba_png(bytes, width, height).is_err(),
            "{width}x{height} with {} bytes must be refused",
            bytes.len()
        );
    }
}

/// A 1 m opening (a tessellated box) voiding the panel through
/// `IfcRelVoidsElement`.
const OPENING_RECORDS: &str = "#30=IFCCARTESIANPOINTLIST3D(((0.4,0.4,-1.),(0.6,0.4,-1.),\
(0.6,0.6,-1.),(0.4,0.6,-1.),(0.4,0.4,1.),(0.6,0.4,1.),(0.6,0.6,1.),(0.4,0.6,1.)),$);
#31=IFCTRIANGULATEDFACESET(#30,$,.T.,((1,2,3),(1,3,4),(5,6,7),(5,7,8),(1,2,6),(1,6,5),(2,3,7),\
(2,7,6),(3,4,8),(3,8,7),(4,1,5),(4,5,8)),$);
#32=IFCSHAPEREPRESENTATION(#2,'Body','Tessellation',(#31));
#33=IFCPRODUCTDEFINITIONSHAPE($,$,(#32));
#34=IFCLOCALPLACEMENT($,#15);
#35=IFCOPENINGELEMENT('2O2Fr$t4X7Zf8NOew3FL02',$,'Opening',$,$,#34,#33,$);
#36=IFCRELVOIDSELEMENT('2O2Fr$t4X7Zf8NOew3FL03',$,$,$,#17,#35);
";

#[test]
fn a_textured_product_cut_by_an_opening_reports_the_void_fallback() {
    let drawing = IfcImporter::new()
        .import(
            &request_bytes(textured_tessellation_with(
                PIXEL_TEXTURE_2X1,
                OPENING_RECORDS,
            )),
            &|| false,
        )
        .unwrap_or_else(|error| panic!("the voided textured fixture must import: {error:?}"));
    let meshes = meshes(&drawing);
    assert_eq!(meshes.len(), 1, "the opening is a cutter, not a product");
    let mesh = meshes[0];
    assert!(
        mesh.texture.is_none(),
        "a CSG cut rebuilds vertices, so the texture cannot be attached"
    );
    assert!(
        mesh.uvs.is_empty(),
        "no texture means no UV set is stored for the cut mesh"
    );
    // The product is still meshed on that path (what the cut itself produced is
    // a CSG question this test does not pin: ifc-lite may leave a tessellated
    // host uncut where it cuts a solid).
    assert!(
        !mesh.triangles.is_empty(),
        "the cut product must still import"
    );
    assert!(
        diagnostic_codes(&drawing).contains(&DIAG_TEXTURED_VOID_FALLBACK),
        "diagnostics: {:?}",
        drawing.report.diagnostics
    );
    assert!(matches!(
        drawing.report.completeness,
        Completeness::Partial(_)
    ));
}
