//! Shared fixtures for the MULTILEADER export tests (P7b-2).
//!
//! The builders are `pub(crate)` so the byte-level tests and the round-trip
//! tests can construct the same definition without duplication. Keeping them
//! here also keeps each test file under the repository's 1000-line limit.

use std::sync::Arc;

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
