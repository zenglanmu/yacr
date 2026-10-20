//! Shared fixtures for the MULTILEADER export tests (P7b-2).
//!
//! The builders are `pub(crate)` so the byte-level tests and the round-trip
//! tests can construct the same definition without duplication. Keeping them
//! here also keeps each test file under the repository's 1000-line limit.

use std::io::Cursor;
use std::sync::Arc;

use acadrust::entities::multileader::LeaderLineBreakInfo;
use acadrust::entities::{
    LeaderContentType, LeaderLinePropertyOverrideFlags as AcadLineOverride,
    MultiLeader as AcadMultiLeader, MultiLeaderPathType as AcadPathType,
    MultiLeaderPropertyOverrideFlags as AcadMask, StartEndPointPair, TextAttachmentDirectionType,
    TextAttachmentPointType, TextAttachmentType,
};
use acadrust::objects::{MultiLeaderStyle as AcadStyle, ObjectType as AcadObjectType};
use acadrust::types::{Handle as AcadHandle, Vector3};
use acadrust::{CadDocument, DxfWriter};
use cad_db::{BlockDefinition, DbEntity, DbObject, DrawingDatabase, DrawingDatabaseBuilder, Layer};

use super::*;
use crate::export::ExportReport;

pub(crate) fn p(x: f64, y: f64, z: f64) -> Point3 {
    Point3 { x, y, z }
}

// ---------------------------------------------------------------------------
// Synthesized definition (the P7b-1 builders, extended to the full model).
// ---------------------------------------------------------------------------

/// A multileader carrying every modelled field: two roots, three lines (one
/// with a per-line arrowhead override and one spline), break data, background
/// and column parameters, a per-line colour/linetype/lineweight, and a
/// non-default plane frame.
pub(crate) fn multileader(style_name: &str, mask: u32, landing_gap: f64) -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 0.3,
            landing_gap,
            arrowhead_size: 0.2,
            text_height: 0.25,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 2.0,
        },
        style_name: style_name.into(),
        // ENABLE_LANDING | ARROWHEAD_SIZE | TEXT_HEIGHT: a nonzero mask whose
        // bits must survive byte for byte.
        override_mask: mask,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "Note".into(),
            location: p(10.0, 2.0, 0.0),
            rotation: 0.25,
            height: 0.25,
            width: 1.5,
            attachment_point: 3,
            left_attachment: 4,
        },
        roots: vec![
            MLeaderRoot {
                leader_index: 0,
                landing_point: p(10.0, 2.0, 0.0),
                dogleg_vector: p(1.0, 0.0, 0.0),
                dogleg_length: 0.3,
            },
            MLeaderRoot {
                leader_index: 1,
                landing_point: p(-4.0, 2.0, 0.0),
                dogleg_vector: p(-1.0, 0.0, 0.0),
                dogleg_length: 0.25,
            },
        ],
        lines: vec![
            MLeaderLine {
                root_index: 0,
                index: 0,
                points: vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0), p(9.0, 1.0, 0.0)],
                path_type: MLeaderPathType::StraightLineSegments,
                override_flags: MLeaderLine::PATH_TYPE,
                arrowhead_size: 0.35,
                arrowhead_handle: None,
            },
            MLeaderLine {
                root_index: 0,
                index: 1,
                points: vec![p(0.0, -3.0, 0.0), p(5.0, -1.0, 0.0)],
                path_type: MLeaderPathType::Spline,
                override_flags: MLeaderLine::ARROWHEAD_SIZE,
                arrowhead_size: 0.4,
                arrowhead_handle: Some(0x2A),
            },
            MLeaderLine {
                root_index: 1,
                index: 0,
                points: vec![p(-6.0, 0.0, 0.0), p(-5.5, 0.5, 0.0), p(-5.0, 1.0, 0.0)],
                path_type: MLeaderPathType::StraightLineSegments,
                override_flags: 0,
                arrowhead_size: 0.2,
                arrowhead_handle: None,
            },
        ],
        reserved: MLeaderReserved {
            roots: vec![
                MLeaderRootExtras {
                    root_index: 0,
                    content_valid: false,
                    unknown: true,
                    text_attachment_direction: 1,
                    breaks: vec![MLeaderBreak {
                        start: p(1.0, 0.0, 0.0),
                        end: p(2.0, 0.0, 0.0),
                    }],
                },
                MLeaderRootExtras {
                    root_index: 1,
                    content_valid: true,
                    unknown: true,
                    text_attachment_direction: 0,
                    breaks: Vec::new(),
                },
            ],
            lines: vec![
                MLeaderLineExtras {
                    root_index: 0,
                    line_index: 0,
                    segment_index: 1,
                    breaks: vec![MLeaderLineBreak {
                        segment_index: 1,
                        points: vec![MLeaderBreak {
                            start: p(2.0, 0.0, 0.0),
                            end: p(3.0, 0.0, 0.0),
                        }],
                    }],
                    break_info_count: 1,
                    legacy_breaks: Vec::new(),
                    line_color: MLeaderColor {
                        index: Some(3),
                        true_color: None,
                    },
                    line_type_handle: Some(0x123),
                    line_weight: 25,
                },
                MLeaderLineExtras {
                    root_index: 0,
                    line_index: 1,
                    segment_index: 0,
                    breaks: Vec::new(),
                    break_info_count: 0,
                    legacy_breaks: Vec::new(),
                    line_color: MLeaderColor {
                        index: Some(0),
                        true_color: None,
                    },
                    line_type_handle: None,
                    line_weight: -1,
                },
                MLeaderLineExtras {
                    root_index: 1,
                    line_index: 2,
                    segment_index: 0,
                    breaks: Vec::new(),
                    break_info_count: 0,
                    legacy_breaks: Vec::new(),
                    line_color: MLeaderColor {
                        index: Some(0),
                        true_color: None,
                    },
                    line_type_handle: None,
                    line_weight: -1,
                },
            ],
            background: Some(MLeaderBackground {
                fill_enabled: true,
                mask_fill_on: true,
                fill_color: MLeaderColor {
                    index: Some(5),
                    true_color: None,
                },
                scale_factor: 1.5,
                transparency: 30,
            }),
            columns: Some(MLeaderColumns {
                column_type: 2,
                width: 4.0,
                gutter: 0.5,
                flow_reversed: true,
                sizes: vec![4.0, 3.0],
            }),
        },
        plane: PlaneFrame {
            origin: p(1.0, 2.0, 0.0),
            u: p(0.0, 1.0, 0.0),
            v: p(-1.0, 0.0, 0.0),
            normal_reversed: true,
        },
    });
    geometry
}

pub(crate) fn rich_multileader() -> MultiLeaderGeometry {
    multileader("Standard", 0x0001_0210, 0.05)
}

/// The second content kind P7b-1 migrates: a leader with no annotation
/// (`172 = 0`).
pub(crate) fn contentless_multileader() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 0.3,
            landing_gap: 0.05,
            arrowhead_size: 0.2,
            text_height: 0.18,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: p(0.0, 0.0, 0.0),
            rotation: 0.0,
            height: 0.18,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(1.0, 0.0, 0.0),
            dogleg_vector: p(1.0, 0.0, 0.0),
            dogleg_length: 0.3,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(0.0, 0.0, 0.0), p(0.5, 0.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.2,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

/// A straight-line, single-root multileader with text: its display is exactly
/// one polyline, one arrowhead mesh, one dogleg line and one text, so a
/// flattened insert placement is observable primitive by primitive.
pub(crate) fn straight_multileader() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 0.3,
            landing_gap: 0.05,
            arrowhead_size: 0.2,
            text_height: 0.18,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::Text,
        text: MLeaderText {
            string: "Note".into(),
            location: p(10.0, 2.0, 0.0),
            rotation: 0.0,
            height: 0.18,
            width: 0.0,
            attachment_point: 1,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(10.0, 2.0, 0.0),
            dogleg_vector: p(1.0, 0.0, 0.0),
            dogleg_length: 0.3,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(0.0, 0.0, 0.0), p(5.0, 0.0, 0.0), p(9.0, 1.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.2,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved::default(),
        plane: PlaneFrame::default(),
    });
    geometry
}

/// A database where `member` is entity 2 inside block 1 and entity 1 is a
/// model-space INSERT of that block with `transform`.
pub(crate) fn db_with_block(member: SemanticGeometry, transform: Transform3) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(cad_db::Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    builder
        .insert_block(BlockDefinition {
            id: BlockId(1),
            entities: vec![EntityId(2)],
            dynamic_visibility: None,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(2),
                type_key: "AcDbMultiLeader".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(2),
            layer: LayerId(0),
            space: SpaceId::Block(BlockId(1)),
            geometry: member,
            draw_order: 0,
        })
        .unwrap();
    builder
        .insert_entity(DbEntity {
            object: DbObject {
                id: ObjectId(1),
                type_key: "AcDbBlockReference".into(),
                revision: Revision(0),
                source_handle: None,
            },
            id: EntityId(1),
            layer: LayerId(0),
            space: SpaceId::Model,
            geometry: SemanticGeometry::Insert {
                block: BlockId(1),
                transform,
            },
            draw_order: 1,
        })
        .unwrap();
    builder.finish().unwrap()
}

pub(crate) fn db_with(geometries: Vec<SemanticGeometry>) -> DrawingDatabase {
    let mut builder = DrawingDatabaseBuilder::new(DatabaseId(1));
    builder
        .insert_layer(Layer {
            id: LayerId(0),
            name: "0".into(),
            visible: true,
        })
        .unwrap();
    builder
        .insert_style(cad_db::Style {
            id: StyleId(0),
            name: "Standard".into(),
            resource_keys: Vec::new(),
        })
        .unwrap();
    for (index, geometry) in geometries.into_iter().enumerate() {
        let id = index as u128 + 1;
        builder
            .insert_entity(DbEntity {
                object: DbObject {
                    id: ObjectId(id),
                    type_key: "AcDbMultiLeader".into(),
                    revision: Revision(0),
                    source_handle: None,
                },
                id: EntityId(id),
                layer: LayerId(0),
                space: SpaceId::Model,
                geometry,
                draw_order: index as i64,
            })
            .unwrap();
    }
    builder.finish().unwrap()
}

pub(crate) fn export_db(database: &DrawingDatabase) -> (Vec<u8>, ExportReport) {
    export::export(&export::ExportRequest {
        database,
        document_id: DocumentId(1),
        space: export::ExportSpace::Model,
        format: export::ExportFormat::DxfText,
        limits: export::ExportLimits::default(),
        units: None,
    })
    .expect("export runs")
}

pub(crate) fn export_one(geometry: SemanticGeometry) -> (Vec<u8>, ExportReport) {
    export_db(&db_with(vec![geometry]))
}

pub(crate) fn import_bytes(bytes: &[u8]) -> DrawingDatabase {
    let request = ImportRequest {
        document: DocumentId(1),
        database: DatabaseId(1),
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
        limits: ImportLimits::default(),
        generation: 0,
    };
    AcadrustImporter::new()
        .import(&request, &|| false)
        .expect("re-import")
        .database
}

// ---------------------------------------------------------------------------
// N7 adversarial source fixtures.
//
// Unlike the definitions above (which are built through our own model and are
// therefore self-consistent), these fixtures start where a real file starts: an
// acadrust `CadDocument` written to DXF, or a domain definition used as the
// exporter's input. Each one deliberately carries a value that the declared
// import/export path normalises away, so the tests can pin that normalisation
// instead of restating the implementation.
// ---------------------------------------------------------------------------

/// The mask the adversarial source carries: ENABLE_LANDING | ARROWHEAD_SIZE |
/// TEXT_HEIGHT. The bits are source data and must survive verbatim.
pub(crate) const ADVERSARIAL_MASK_BITS: u32 = 0x0001_0210;

/// The style object both adversarial entities point at.
fn adversarial_style(doc: &mut CadDocument) -> AcadHandle {
    let handle = doc.allocate_handle();
    let mut style = AcadStyle::new("Adversarial");
    style.handle = handle;
    // The declared content-type normalisation does not consult the style's
    // default text. It is explicitly empty here so the fixture cannot hide
    // behind a seed value.
    style.default_text = String::new();
    style.path_type = acadrust::objects::MultiLeaderPathType::StraightLineSegments;
    style.enable_landing = true;
    style.enable_dogleg = true;
    style.landing_distance = 0.3;
    style.landing_gap = 0.05;
    style.arrowhead_size = 0.2;
    style.text_height = 0.25;
    doc.objects
        .insert(handle, AcadObjectType::MultiLeaderStyle(style));
    handle
}

/// A two-root, three-line source whose entity-level attachment direction (`271`)
/// disagrees with root 0, and whose break records deliberately carry bogus
/// legacy mirror fields.
fn source_rich_multileader(style: AcadHandle) -> AcadMultiLeader {
    let mut ml = AcadMultiLeader::new();
    ml.style_handle = Some(style);
    ml.content_type = LeaderContentType::MText;
    ml.property_override_flags = AcadMask::from_bits_retain(ADVERSARIAL_MASK_BITS);
    ml.path_type = AcadPathType::StraightLineSegments;
    ml.enable_landing = true;
    ml.enable_dogleg = true;
    ml.dogleg_length = 0.3;
    ml.arrowhead_size = 0.2;
    ml.text_height = 0.25;
    ml.scale_factor = 2.0;
    ml.text_attachment_point = TextAttachmentPointType::Right;
    ml.text_left_attachment = TextAttachmentType::BottomOfBottomLine;
    // Adversarial: the entity-level direction is Vertical while root 0 says
    // Horizontal. The model stores only the root value, so the export must
    // re-derive the entity-tail `271` from root 0.
    ml.text_attachment_direction = TextAttachmentDirectionType::Vertical;

    ml.context.scale_factor = 2.0;
    ml.context.text_height = 0.25;
    ml.context.arrowhead_size = 0.2;
    ml.context.landing_gap = 0.05;
    ml.context.content_base_point = Vector3::new(10.0, 2.0, 0.0);
    ml.context.has_text_contents = true;
    ml.context.text_string = "Adversarial".into();
    ml.context.text_location = Vector3::new(10.0, 2.0, 0.0);
    ml.context.text_rotation = 0.25;
    ml.context.text_width = 1.5;
    ml.context.text_normal = Vector3::UNIT_Z;
    ml.context.text_direction = Vector3::UNIT_X;
    ml.context.base_point = Vector3::new(1.0, 2.0, 0.0);
    ml.context.base_direction = Vector3::new(0.0, 1.0, 0.0);
    ml.context.base_vertical = Vector3::new(-1.0, 0.0, 0.0);
    ml.context.normal_reversed = true;

    let root = ml.add_leader_root();
    root.leader_index = 0;
    root.connection_point = Vector3::new(10.0, 2.0, 0.0);
    root.direction = Vector3::new(1.0, 0.0, 0.0);
    root.landing_distance = 0.3;
    root.text_attachment_direction = TextAttachmentDirectionType::Horizontal;
    let line = root.create_line(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(5.0, 0.0, 0.0),
        Vector3::new(9.0, 1.0, 0.0),
    ]);
    line.index = 0;
    line.path_type = AcadPathType::StraightLineSegments;
    line.override_flags = AcadLineOverride::PATH_TYPE;
    line.arrowhead_size = 0.35;
    // Two complete break records (segments 3 and 4) plus legacy mirrors that
    // disagree with them. acadrust's DXF reader derives the mirrors, so this
    // inconsistency is erased on the source read; the exporter must not write
    // the count/segment values as independent fields either.
    line.break_infos = vec![
        LeaderLineBreakInfo {
            segment_index: 3,
            break_points: vec![StartEndPointPair::new(
                Vector3::new(2.0, 0.0, 0.0),
                Vector3::new(3.0, 0.0, 0.0),
            )],
        },
        LeaderLineBreakInfo {
            segment_index: 4,
            break_points: vec![StartEndPointPair::new(
                Vector3::new(4.0, 0.0, 0.0),
                Vector3::new(4.5, 0.0, 0.0),
            )],
        },
    ];
    line.break_info_count = 99;
    line.segment_index = 7;

    let line = root.create_line(vec![
        Vector3::new(0.0, -3.0, 0.0),
        Vector3::new(5.0, -1.0, 0.0),
    ]);
    line.index = 1;
    line.path_type = AcadPathType::Spline;
    line.override_flags = AcadLineOverride::ARROWHEAD_SIZE;
    line.arrowhead_size = 0.4;
    line.arrowhead_handle = Some(AcadHandle::new(0x2A));

    let root = ml.add_leader_root();
    root.leader_index = 1;
    root.connection_point = Vector3::new(-4.0, 2.0, 0.0);
    root.direction = Vector3::new(-1.0, 0.0, 0.0);
    root.landing_distance = 0.25;
    root.text_attachment_direction = TextAttachmentDirectionType::Vertical;
    let line = root.create_line(vec![
        Vector3::new(-6.0, 0.0, 0.0),
        Vector3::new(-5.5, 0.5, 0.0),
        Vector3::new(-5.0, 1.0, 0.0),
    ]);
    line.index = 0;
    line.path_type = AcadPathType::StraightLineSegments;
    line.override_flags = AcadLineOverride::NONE;
    line.arrowhead_size = 0.2;

    ml
}

/// A source that declares MTEXT content and `has_text_contents = true` but
/// carries an **empty** string. The import path normalises it to no content.
fn source_empty_text_multileader(style: AcadHandle) -> AcadMultiLeader {
    let mut ml = AcadMultiLeader::new();
    ml.style_handle = Some(style);
    ml.content_type = LeaderContentType::MText;
    ml.path_type = AcadPathType::StraightLineSegments;
    ml.enable_landing = true;
    ml.enable_dogleg = true;
    ml.dogleg_length = 0.3;
    ml.arrowhead_size = 0.2;
    ml.text_height = 0.25;
    ml.scale_factor = 2.0;

    ml.context.scale_factor = 2.0;
    ml.context.text_height = 0.25;
    ml.context.arrowhead_size = 0.2;
    ml.context.landing_gap = 0.05;
    ml.context.text_location = Vector3::new(10.0, 2.0, 0.0);
    // The two adversarial source values: the flag says there is text content,
    // the string says there is none.
    ml.context.has_text_contents = true;
    ml.context.text_string = String::new();
    ml.context.base_point = Vector3::ZERO;
    ml.context.base_direction = Vector3::UNIT_X;
    ml.context.base_vertical = Vector3::UNIT_Y;

    let root = ml.add_leader_root();
    root.leader_index = 0;
    root.connection_point = Vector3::new(10.0, 2.0, 0.0);
    root.landing_distance = 0.3;
    let line = root.create_line(vec![
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(9.0, 1.0, 0.0),
    ]);
    line.index = 0;
    line.path_type = AcadPathType::StraightLineSegments;
    line.arrowhead_size = 0.2;

    ml
}

/// The adversarial source file: the rich entity plus the empty-text entity.
fn adversarial_source_document() -> CadDocument {
    let mut doc = CadDocument::new();
    let style = adversarial_style(&mut doc);
    doc.add_entity(EntityType::MultiLeader(Box::new(source_rich_multileader(
        style,
    ))))
    .expect("the rich source entity");
    doc.add_entity(EntityType::MultiLeader(Box::new(
        source_empty_text_multileader(style),
    )))
    .expect("the empty-text source entity");
    doc
}

pub(crate) fn adversarial_source_bytes() -> Vec<u8> {
    let document = adversarial_source_document();
    DxfWriter::new(&document)
        .write_to_vec()
        .expect("write the adversarial source")
}

pub(crate) fn read_dxf(bytes: &[u8]) -> CadDocument {
    acadrust::DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .expect("DXF reader")
        .read()
        .expect("the DXF parses")
}

pub(crate) fn acad_multileaders(document: &CadDocument) -> Vec<AcadMultiLeader> {
    document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn imported_geometries(database: &DrawingDatabase) -> Vec<MultiLeaderGeometry> {
    database
        .entities()
        .filter_map(|entity| match &entity.geometry {
            SemanticGeometry::MultiLeader(leader) => Some(leader.as_ref().clone()),
            _ => None,
        })
        .collect()
}

pub(crate) fn rich_source(leaders: &[AcadMultiLeader]) -> &AcadMultiLeader {
    leaders
        .iter()
        .find(|leader| leader.context.leader_roots.len() == 2)
        .expect("the two-root source")
}

pub(crate) fn empty_text_source(leaders: &[AcadMultiLeader]) -> &AcadMultiLeader {
    leaders
        .iter()
        .find(|leader| {
            leader.context.leader_roots.len() == 1 && leader.context.text_string.is_empty()
        })
        .expect("the empty-text source")
}

pub(crate) fn rich_geometry(geometries: &[MultiLeaderGeometry]) -> &MultiLeaderGeometry {
    geometries
        .iter()
        .find(|geometry| geometry.text().string == "Adversarial")
        .expect("the rich imported geometry")
}

pub(crate) fn empty_text_geometry(geometries: &[MultiLeaderGeometry]) -> &MultiLeaderGeometry {
    geometries
        .iter()
        .find(|geometry| geometry.content() == MLeaderContent::None)
        .expect("the empty-text imported geometry")
}

/// Rebuild `geometry` with deliberately inconsistent legacy break mirrors.
///
/// This cannot be done through acadrust's public API: its DXF and DWG readers
/// derive `break_info_count`/`segment_index` from the break records, so a
/// written source file can never carry a disagreeing mirror. The domain model is
/// the exporter's input, so the inconsistency is injected there — the closest
/// thing that can be constructed without faking an acadrust fixture.
pub(crate) fn with_inconsistent_break_mirrors(
    geometry: &MultiLeaderGeometry,
) -> MultiLeaderGeometry {
    let mut reserved = geometry.reserved().clone();
    let line = reserved
        .lines
        .iter_mut()
        .find(|line| line.breaks.len() == 2)
        .expect("the rich source carries two break records");
    line.segment_index = 7;
    line.break_info_count = 99;
    let (rebuilt, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: geometry.style().clone(),
        style_name: geometry.style_name().to_string(),
        override_mask: geometry.override_mask(),
        content: geometry.content(),
        text: geometry.text().clone(),
        roots: geometry.roots().to_vec(),
        lines: geometry.lines().to_vec(),
        reserved,
        plane: geometry.plane(),
    });
    rebuilt
}

/// A minimal domain definition whose legacy break mirrors disagree with its two
/// complete break records: the count says 99 and the first segment says 7, while
/// the records say 2 breaks starting on segment 3.
pub(crate) fn source_model_with_inconsistent_break_mirrors() -> MultiLeaderGeometry {
    let (geometry, _) = MultiLeaderGeometry::from_import(MultiLeaderParts {
        style: MLeaderStyleValues {
            path_type: MLeaderPathType::StraightLineSegments,
            enable_landing: true,
            enable_dogleg: true,
            landing_distance: 0.3,
            landing_gap: 0.05,
            arrowhead_size: 0.2,
            text_height: 0.25,
            text_style: StyleId(0),
            text_style_name: "Standard".into(),
            text_font: None,
            scale_factor: 1.0,
        },
        style_name: "Standard".into(),
        override_mask: 0,
        content: MLeaderContent::None,
        text: MLeaderText {
            string: String::new(),
            location: p(0.0, 0.0, 0.0),
            rotation: 0.0,
            height: 0.25,
            width: 0.0,
            attachment_point: 2,
            left_attachment: 2,
        },
        roots: vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(2.0, 0.0, 0.0),
            dogleg_vector: p(1.0, 0.0, 0.0),
            dogleg_length: 0.3,
        }],
        lines: vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(0.0, 0.0, 0.0), p(1.0, 0.0, 0.0), p(2.0, 0.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 0,
            arrowhead_size: 0.2,
            arrowhead_handle: None,
        }],
        reserved: MLeaderReserved {
            roots: Vec::new(),
            lines: vec![MLeaderLineExtras {
                root_index: 0,
                line_index: 0,
                // The bogus mirrors.
                segment_index: 7,
                breaks: vec![
                    MLeaderLineBreak {
                        segment_index: 3,
                        points: vec![MLeaderBreak {
                            start: p(0.5, 0.0, 0.0),
                            end: p(0.6, 0.0, 0.0),
                        }],
                    },
                    MLeaderLineBreak {
                        segment_index: 4,
                        points: vec![MLeaderBreak {
                            start: p(1.2, 0.0, 0.0),
                            end: p(1.4, 0.0, 0.0),
                        }],
                    },
                ],
                break_info_count: 99,
                legacy_breaks: Vec::new(),
                line_color: MLeaderColor {
                    index: Some(0),
                    true_color: None,
                },
                line_type_handle: None,
                line_weight: -1,
            }],
            background: None,
            columns: None,
        },
        plane: PlaneFrame::default(),
    });
    geometry
}
