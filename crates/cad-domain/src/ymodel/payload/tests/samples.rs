//! Payload samples, one per semantic geometry variant plus the nested
//! alternatives the one-per-class samples do not reach.
//!
//! Split out of `tests/mod.rs` for the code-size ceiling. The samples are built
//! through the restore constructors wherever the validating constructors would
//! refuse synthetic data (`DimensionGeometry`, `HatchGeometry`,
//! `MultiLeaderGeometry`, `TableGeometry`) — the same path a document load
//! takes.

use super::{origin, p, z_axis};
use crate::dimension::DisplaySource;
use crate::{
    BlockId, Body3d, BodyDisplay, BodyExact, BodyKind, BoundaryEdge, BrepCurve, BrepData, BrepFace,
    BrepLoop, BrepPlacement, BrepShell, BrepSurface, Completeness, DimensionDefinition,
    DimensionGeometry, DimensionKind, DimensionPoints, DimensionStyleValues, HatchFill,
    HatchGeometry, HatchGradient, HatchLoop, HatchLoopFlags, HatchPatternType, HatchPlane,
    HatchStyle, ImageAdjust, ImageClip, MLeaderBackground, MLeaderBlockContent, MLeaderBreak,
    MLeaderColor, MLeaderColumns, MLeaderContent, MLeaderLine, MLeaderLineBreak, MLeaderLineExtras,
    MLeaderPathType, MLeaderReserved, MLeaderRoot, MLeaderRootExtras, MLeaderStyleValues,
    MLeaderText, MLeaderTextAngle, Mesh, MultiLeaderGeometry, PlaneFrame, Precision, Revision,
    SemanticGeometry, StyleId, SubElementId, TableBorder, TableCellAlignH, TableCellAlignV,
    TableCellAlignment, TableCellBorders, TableCellGeometry, TableCellMargins, TableColumnGeometry,
    TableDisplayAppearance, TableFrame, TableGeometry, TableRowGeometry, TableRowStyle,
    TableStyleValues, TextAlignH, TextAlignV, TextLineSpacing, TextLineSpacingStyle,
};

// ---------------------------------------------------------------------------
// Samples for the analytic classes.
// ---------------------------------------------------------------------------

pub(super) fn mesh_sample() -> Mesh {
    Mesh {
        vertices: vec![origin(), p(1.0, 0.0, 0.0), p(0.0, 1.0, 0.0)],
        triangles: vec![[0, 1, 2]],
        normals: vec![z_axis(); 3],
        face_sources: vec![
            None,
            Some(SubElementId {
                source_key: "face:1".to_string(),
                topology_revision: Revision(7),
            }),
            Some(SubElementId {
                source_key: "face:2".to_string(),
                topology_revision: Revision(9),
            }),
        ],
        colors: vec![[1, 2, 3], [4, 5, 6], [7, 8, 9]],
        // One UV per vertex plus a canonical texture key (ADR-0038 sample).
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        texture: Some(super::TEXTURE_KEY.to_string()),
    }
}

pub(super) fn brep_sample() -> BrepData {
    let loop_ = BrepLoop {
        edges: vec![
            BrepCurve::Line {
                start: origin(),
                end: p(1.0, 0.0, 0.0),
            },
            BrepCurve::Circle {
                center: origin(),
                normal: z_axis(),
                u_dir: p(1.0, 0.0, 0.0),
                radius: 2.0,
            },
            BrepCurve::Arc {
                center: origin(),
                normal: z_axis(),
                u_dir: p(1.0, 0.0, 0.0),
                radius: 3.0,
                start: p(3.0, 0.0, 0.0),
                end: p(0.0, 3.0, 0.0),
            },
            BrepCurve::Unsupported {
                type_key: "acis:bs3_curve".to_string(),
            },
        ],
    };
    let surface_of = |surface| BrepFace {
        id: 1,
        surface,
        reversed: true,
        loops: vec![BrepLoop {
            edges: loop_.edges.clone(),
        }],
        domain: Some([[0.0, 1.0], [-1.0, 1.0]]),
        tolerance: Some(1e-6),
    };
    BrepData {
        shells: vec![BrepShell {
            id: 4,
            faces: vec![
                surface_of(BrepSurface::Plane {
                    origin: origin(),
                    normal: z_axis(),
                    u_dir: p(1.0, 0.0, 0.0),
                }),
                surface_of(BrepSurface::Sphere {
                    center: origin(),
                    radius: 5.0,
                    u_dir: p(1.0, 0.0, 0.0),
                    pole: z_axis(),
                }),
                surface_of(BrepSurface::Torus {
                    center: origin(),
                    normal: z_axis(),
                    major_radius: 5.0,
                    minor_radius: 1.0,
                    u_dir: p(1.0, 0.0, 0.0),
                }),
                surface_of(BrepSurface::Cylinder {
                    origin: origin(),
                    axis: z_axis(),
                    ref_dir: p(1.0, 0.0, 0.0),
                    radius: 2.0,
                }),
                surface_of(BrepSurface::Cone {
                    origin: origin(),
                    axis: z_axis(),
                    ref_dir: p(1.0, 0.0, 0.0),
                    radius: 2.0,
                    sin_half_angle: 0.5,
                    cos_half_angle: 0.866_025_403_784_438_6,
                }),
                surface_of(BrepSurface::Nurbs {
                    u_knots: vec![0.0, 0.0, 1.0, 1.0],
                    v_knots: vec![0.0, 1.0],
                    points: vec![
                        vec![[1.0, 0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0]],
                        vec![[0.0, 0.0, 1.0, 2.0], [0.0, 1.0, 1.0, 2.0]],
                    ],
                }),
                surface_of(BrepSurface::Unsupported {
                    type_key: "acis:spline_surface".to_string(),
                }),
            ],
        }],
        placement: Some(BrepPlacement {
            matrix: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            translation: p(10.0, 20.0, 30.0),
            scale: 2.0,
        }),
        tolerance: Some(0.001),
    }
}

pub(super) fn body_sample() -> Body3d {
    Body3d::from_persisted(
        BodyKind::Solid,
        BodyExact::Exact(Box::new(brep_sample())),
        BodyDisplay {
            mesh: mesh_sample(),
            edges: vec![vec![origin(), p(1.0, 1.0, 1.0)]],
            tolerance: Some(0.5),
            precision: Precision::Approximate {
                error_bound: Some(0.125),
            },
            completeness: Completeness::Partial(vec!["face 2 is unsupported".to_string()]),
        },
    )
}

pub(super) fn text_sample() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "π ±0.05".to_string(),
        position: p(1.0, 2.0, 3.0),
        style: StyleId(2),
        height: 2.5,
        rotation: 0.25,
        font: Some("arial.ttf".to_string()),
        h_align: TextAlignH::Right,
        v_align: TextAlignV::Middle,
        wrap_width: Some(40.0),
        width_factor: 0.8,
        line_spacing: TextLineSpacing {
            factor: 2.0,
            style: TextLineSpacingStyle::Exactly,
        },
    }
}

pub(super) fn image_sample() -> SemanticGeometry {
    SemanticGeometry::Image {
        origin: p(1.0, 1.0, 0.0),
        u: p(0.5, 0.0, 0.0),
        v: p(0.0, 0.5, 0.0),
        pixels: [1024.0, 768.0],
        file: Some("logo.png".to_string()),
        clip: Some(ImageClip {
            vertices: vec![[0.0, 0.0], [1.0, 1.0]],
            inside: false,
        }),
        visible: false,
        adjust: ImageAdjust {
            brightness: 70,
            contrast: 30,
            fade: 25,
            monochrome: true,
        },
    }
}

// ---------------------------------------------------------------------------
// Samples for the four families with derived display caches.
// ---------------------------------------------------------------------------

pub(super) fn display_child() -> Vec<SemanticGeometry> {
    vec![
        SemanticGeometry::Line {
            start: origin(),
            end: p(1.0, 0.0, 0.0),
        },
        SemanticGeometry::Point(p(2.0, 0.0, 0.0)),
    ]
}

pub(super) fn dimension_style_sample() -> DimensionStyleValues {
    DimensionStyleValues {
        arrow_size: 2.5,
        ext_offset: 0.625,
        ext_beyond: 1.25,
        text_height: 2.5,
        text_gap: 0.625,
        center_mark: -2.5,
        dim_line_increment: 3.75,
        text_vertical: 1,
        text_vertical_position: 0.5,
        suppress_ext1: true,
        suppress_ext2: false,
        suppress_dim1: false,
        suppress_dim2: true,
        linear_factor: 2.0,
        decimals: 4,
        rounding: 0.05,
        postfix: "<> mm".to_string(),
        zero_suppression: 12,
        linear_unit_format: 2,
        decimal_separator: 46,
        text_style_name: "Standard".to_string(),
        text_style: StyleId(3),
        text_font: Some("romans.shx".to_string()),
    }
}

pub(super) fn dimension_sample() -> DimensionGeometry {
    DimensionGeometry::from_persisted(
        DimensionDefinition {
            kind: DimensionKind::Linear { rotation: 0.5 },
            points: DimensionPoints::Linear {
                first: origin(),
                second: p(10.0, 0.0, 0.0),
                dimension_line: p(5.0, 3.0, 0.0),
            },
            text_middle: p(5.0, 4.0, 0.0),
            text_rotation: 0.1,
            normal: z_axis(),
        },
        "Standard".to_string(),
        dimension_style_sample(),
        2.0,
        Some("<> ±0.1".to_string()),
        display_child(),
        TextAlignH::Center,
        DisplaySource::Picture,
    )
}

pub(super) fn boundary_edges() -> Vec<BoundaryEdge> {
    vec![
        BoundaryEdge::Line {
            start: [0.0, 0.0],
            end: [1.0, 0.0],
        },
        BoundaryEdge::CircularArc {
            center: [0.0, 0.0],
            radius: 1.0,
            start_angle: 0.0,
            end_angle: 1.5,
            ccw: true,
        },
        BoundaryEdge::EllipticArc {
            center: [0.0, 0.0],
            major_axis: [2.0, 0.0],
            ratio: 0.5,
            start_angle: 0.0,
            end_angle: 1.0,
            ccw: false,
        },
        BoundaryEdge::Spline {
            degree: 3,
            rational: true,
            periodic: false,
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            control_points: vec![[0.0, 0.0, 1.0], [1.0, 1.0, 1.0]],
            fit_points: vec![[0.0, 0.0], [1.0, 1.0]],
            start_tangent: [1.0, 0.0],
            end_tangent: [0.0, 1.0],
        },
    ]
}

pub(super) fn hatch_loop_sample() -> HatchLoop {
    HatchLoop {
        points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0], [0.0, 3.0]],
        bulges: vec![0.0, 0.25, 0.0, 0.0],
        flags: HatchLoopFlags(HatchLoopFlags::EXTERNAL),
        source_edges: Some(boundary_edges()),
    }
}

pub(super) fn hatch_sample() -> HatchGeometry {
    HatchGeometry::from_persisted(
        vec![hatch_loop_sample()],
        HatchPlane::new(z_axis(), 2.5),
        HatchStyle::Outer,
        HatchFill::Gradient(HatchGradient::SingleColor {
            name: "LINEAR".to_string(),
            reserved: 1,
            angle_raw: 0.75,
            shift: 0.25,
            tint: -0.5,
            stops: vec![crate::hatch::GradientStop {
                value: 0.0,
                rgb: [255, 0, 0],
            }],
        }),
        vec![[0.5, 0.5], [1.5, 1.5]],
        Some(0.001),
        HatchPatternType::Custom,
        [0.125, 0.25],
        display_child(),
    )
}

pub(super) fn mleader_style_sample() -> MLeaderStyleValues {
    MLeaderStyleValues {
        path_type: MLeaderPathType::Spline,
        enable_landing: true,
        enable_dogleg: true,
        landing_distance: 5.0,
        landing_gap: 0.5,
        arrowhead_size: 2.5,
        text_height: 3.0,
        text_style: StyleId(4),
        text_style_name: "Annotative".to_string(),
        text_font: Some("arial.ttf".to_string()),
        scale_factor: 1.5,
        text_angle_type: MLeaderTextAngle::Optimized,
    }
}

pub(super) fn mleader_sample() -> MultiLeaderGeometry {
    MultiLeaderGeometry::from_persisted(
        mleader_style_sample(),
        "Standard".to_string(),
        0x2000,
        MLeaderContent::Block(MLeaderBlockContent {
            block: Some(BlockId(6)),
            name: "TAG".to_string(),
            handle: Some(0x1f),
            base_point: Some(p(0.5, 0.5, 0.0)),
            location: p(10.0, 10.0, 0.0),
            scale: p(2.0, 2.0, 1.0),
            rotation: 0.5,
            connection: 1,
        }),
        MLeaderText {
            string: "NOTE".to_string(),
            location: p(10.0, 10.0, 0.0),
            rotation: 0.1,
            height: 3.0,
            width: 40.0,
            attachment_point: 2,
            left_attachment: 5,
        },
        vec![MLeaderRoot {
            leader_index: 0,
            landing_point: p(10.0, 10.0, 0.0),
            dogleg_vector: p(1.0, 0.0, 0.0),
            dogleg_length: 5.0,
        }],
        vec![MLeaderLine {
            root_index: 0,
            index: 0,
            points: vec![p(0.0, 0.0, 0.0), p(5.0, 5.0, 0.0)],
            path_type: MLeaderPathType::StraightLineSegments,
            override_flags: 17,
            arrowhead_size: 2.5,
            arrowhead_handle: Some(0x2a),
        }],
        MLeaderReserved {
            roots: vec![MLeaderRootExtras {
                root_index: 0,
                content_valid: true,
                unknown: false,
                text_attachment_direction: 1,
                breaks: vec![MLeaderBreak {
                    start: p(1.0, 1.0, 0.0),
                    end: p(2.0, 2.0, 0.0),
                }],
            }],
            lines: vec![MLeaderLineExtras {
                root_index: 0,
                line_index: 0,
                segment_index: 1,
                breaks: vec![MLeaderLineBreak {
                    segment_index: 1,
                    points: vec![MLeaderBreak {
                        start: p(3.0, 3.0, 0.0),
                        end: p(4.0, 4.0, 0.0),
                    }],
                }],
                break_info_count: 1,
                legacy_breaks: Vec::new(),
                line_color: MLeaderColor {
                    index: Some(7),
                    true_color: Some(0x00ff_00ff),
                },
                line_type_handle: Some(0x33),
                line_weight: 25,
            }],
            background: Some(MLeaderBackground {
                fill_enabled: true,
                mask_fill_on: false,
                fill_color: MLeaderColor {
                    index: None,
                    true_color: Some(0x0012_3456),
                },
                scale_factor: 1.5,
                transparency: 20,
            }),
            columns: Some(MLeaderColumns {
                column_type: 2,
                width: 10.0,
                gutter: 1.0,
                flow_reversed: true,
                sizes: vec![10.0, 12.0],
            }),
        },
        PlaneFrame {
            origin: p(1.0, 2.0, 3.0),
            u: p(0.0, 1.0, 0.0),
            v: p(-1.0, 0.0, 0.0),
            normal_reversed: true,
        },
        display_child(),
    )
}

pub(super) fn table_cell_sample() -> TableCellGeometry {
    TableCellGeometry {
        text: "Q1".to_string(),
        text_height: 2.5,
        style_name: "Standard".to_string(),
        style: StyleId(5),
        font: Some("arial.ttf".to_string()),
        alignment: TableCellAlignment {
            horizontal: TableCellAlignH::Right,
            vertical: TableCellAlignV::Bottom,
        },
        rotation: 0.125,
        merge_width: 2,
        merge_height: 1,
        borders: TableCellBorders {
            top: TableBorder {
                visible: true,
                double_spacing: 0.1,
                color: Some([1, 2, 3]),
                lineweight_hundredths: Some(25),
            },
            right: TableBorder::VISIBLE,
            bottom: TableBorder::HIDDEN,
            left: TableBorder::default(),
        },
        text_color: Some([10, 20, 30]),
        fill_color: Some([40, 50, 60]),
    }
}

pub(super) fn table_row_style_sample() -> TableRowStyle {
    TableRowStyle {
        text_height: 3.0,
        text_style_name: "Title".to_string(),
        alignment: TableCellAlignment::DEFAULT,
        borders: TableCellBorders {
            top: TableBorder::HIDDEN,
            right: TableBorder::VISIBLE,
            bottom: TableBorder::VISIBLE,
            left: TableBorder::VISIBLE,
        },
        text_color: Some([7, 8, 9]),
        fill_color: None,
    }
}

pub(super) fn table_style_sample() -> TableStyleValues {
    TableStyleValues {
        name: "Standard".to_string(),
        description: "default table".to_string(),
        horizontal_margin: 0.06,
        vertical_margin: 0.06,
        text_height: 2.5,
        text_style_name: "Standard".to_string(),
        borders: TableCellBorders {
            top: TableBorder::VISIBLE,
            right: TableBorder::VISIBLE,
            bottom: TableBorder::VISIBLE,
            left: TableBorder::VISIBLE,
        },
        data_row_alignment: TableCellAlignment::DEFAULT,
        text_color: None,
        fill_color: Some([200, 200, 200]),
        title_row: Some(table_row_style_sample()),
        header_row: None,
        title_suppressed: false,
        header_suppressed: true,
    }
}

pub(super) fn table_sample() -> TableGeometry {
    TableGeometry::from_persisted(
        TableFrame {
            origin: p(1.0, 2.0, 0.0),
            horizontal: p(1.0, 0.0, 0.0),
            normal: z_axis(),
        },
        vec![
            TableColumnGeometry {
                width: 20.0,
                name: "A".to_string(),
            },
            TableColumnGeometry {
                width: 30.0,
                name: "B".to_string(),
            },
        ],
        vec![
            TableRowGeometry {
                height: 5.0,
                cells: vec![table_cell_sample()],
            },
            TableRowGeometry {
                height: 6.0,
                cells: vec![
                    TableCellGeometry {
                        text: String::new(),
                        ..table_cell_sample()
                    },
                    TableCellGeometry {
                        text: "Q2".to_string(),
                        merge_width: 1,
                        ..table_cell_sample()
                    },
                ],
            },
        ],
        table_style_sample(),
        vec![
            ((0, 1), TableCellMargins::uniform(0.2)),
            (
                (1, 0),
                TableCellMargins {
                    left: Some(0.1),
                    top: None,
                    right: Some(0.3),
                    bottom: None,
                },
            ),
        ],
        "Schedule".to_string(),
        "linked table".to_string(),
        display_child(),
        vec![
            TableDisplayAppearance::border(Some([1, 2, 3]), Some(25)),
            TableDisplayAppearance::NONE,
        ],
    )
}
