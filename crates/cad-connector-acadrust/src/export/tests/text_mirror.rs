//! `MIRRTEXT` glyph-mirror round-trip (DXF TEXT group 71 / MTEXT normal).

use super::entities::export_one_as;
use super::*;

fn mirrored_text_geometry() -> SemanticGeometry {
    SemanticGeometry::Text {
        text: "ABC".into(),
        position: p(1.0, 2.0, 0.0),
        style: StyleId(0),
        height: 2.5,
        rotation: 0.0,
        font: None,
        h_align: TextAlignH::Left,
        v_align: TextAlignV::Top,
        wrap_width: None,
        width_factor: 1.0,
        line_spacing: cad_domain::TextLineSpacing::default(),
        mirrored: true,
    }
}

#[test]
fn mirrored_text_writes_dxf_generation_bit_two_and_reimports_mirrored() {
    let (bytes, report) = export_one_as("AcDbText", mirrored_text_geometry());
    assert_eq!(report.completeness, Completeness::Complete);
    match &read_entities(&bytes)[0] {
        EntityType::Text(text) => {
            assert_eq!(
                text.generation_flags & 0x2,
                0x2,
                "DXF group 71 bit 2 is the backward/mirror flag"
            );
            assert_eq!(text.generation_flags & 0x4, 0, "upside-down is not written");
        }
        other => panic!("expected TEXT, got {other:?}"),
    }
    let reimported = import_bytes(&bytes);
    let mirrored = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { mirrored, .. } => Some(*mirrored),
            _ => None,
        });
    assert_eq!(mirrored, Some(true), "the flag round-trips through DXF");
}

#[test]
fn mirrored_mtext_uses_the_oriented_normal_and_reimports_mirrored() {
    let (bytes, report) = export_one_as("AcDbMText", mirrored_text_geometry());
    assert_eq!(report.completeness, Completeness::Complete);
    match &read_entities(&bytes)[0] {
        EntityType::MText(text) => {
            assert!(
                (text.normal - Vector3::new(0.0, 0.0, -1.0)).length() < 1e-12,
                "MTEXT has no generation flags; the mirrored frame is the -Z normal"
            );
        }
        other => panic!("expected MTEXT, got {other:?}"),
    }
    let reimported = import_bytes(&bytes);
    let mirrored = reimported
        .entities()
        .find_map(|entity| match &entity.geometry {
            SemanticGeometry::Text { mirrored, .. } => Some(*mirrored),
            _ => None,
        });
    assert_eq!(
        mirrored,
        Some(true),
        "the -Z normal round-trips as mirrored"
    );
}
