//! The ACIS export-version matrix: which container/payload combinations are
//! accepted, which are refused by name, and what the produced bytes carry.
//!
//! Everything here is **our own writer read back by our own readers**
//! (`acadrust`'s SAT/SAB parser and this crate's lift to a neutral B-rep). No
//! third-party CAD reader has opened any of these bytes, so none of this is a
//! vendor-compatibility claim — including the ASM 21800 payload and every
//! AC1027/AC1032 `ACDSDATA` container.

use super::acis_fixtures::{assert_brep_equivalent, box_brep, solid_body, wedge_brep};
use super::*;
use crate::export::ExportOptions;

/// The SAB/ASM magic this exporter's payloads start with.
const SAB_MAGIC: &[u8] = b"ACIS BinaryFile";

/// The authored box the matrix tests export: 1 × 2 × 3, one shell, six faces,
/// twelve edges.
fn authored_box() -> BrepData {
    box_brep(1.0, 2.0, 3.0)
}

fn export_with(
    db: &DrawingDatabase,
    format: ExportFormat,
    acis_version: AcisVersion,
    dxf_target: DxfTarget,
) -> CadResult<(Vec<u8>, ExportReport)> {
    export(&ExportRequest {
        database: db,
        document_id: DocumentId(1),
        space: ExportSpace::Model,
        format,
        limits: ExportLimits::default(),
        units: None,
        acis_version,
        dxf_target,
    })
}

fn box_database() -> DrawingDatabase {
    db_with(vec![solid_body(authored_box())])
}

/// The SAB header version number (bytes 15..19, little endian) of a payload.
fn sab_version(blob: &[u8]) -> u32 {
    assert!(
        blob.starts_with(SAB_MAGIC),
        "a SAB/ASM blob starts with its magic: {:?}",
        &blob[..blob.len().min(16)]
    );
    u32::from_le_bytes(blob[15..19].try_into().expect("the version field fits"))
}

/// The SAB header flags (bytes 27..31, little endian). ShapeManager (ASM)
/// payloads set bits 2 and 3 (`12`).
fn sab_flags(blob: &[u8]) -> u32 {
    u32::from_le_bytes(blob[27..31].try_into().expect("the flags field fits"))
}

fn decode_hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0, "a 310 chunk is whole bytes: {text}");
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                other => panic!("not a hex digit: {}", other as char),
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect()
}

/// The binary payload the DXF writer queued into `ACDSDATA` (the `310` chunks).
fn acds_blob(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    assert!(
        text.contains("ACDSDATA"),
        "a R2013/R2018 DXF carries an ACDSDATA section"
    );
    assert!(
        text.contains("ACDSRECORD"),
        "the payload is an ACDSRECORD with an ASM_Data field"
    );
    let mut blob = Vec::new();
    for (code, value) in dxf_pairs(bytes) {
        if code == 310 {
            blob.extend_from_slice(&decode_hex(&value));
        }
    }
    assert!(!blob.is_empty(), "ACDSDATA carries at least one 310 chunk");
    blob
}

/// `(shells, faces, edges)` records of a parsed SAT/SAB document.
fn topology_counts(document: &acadrust::entities::acis::SatDocument) -> (usize, usize, usize) {
    (
        document.records_of_type("shell").len(),
        document.records_of_type("face").len(),
        document.records_of_type("edge").len(),
    )
}

/// The one `3DSOLID` of a read-back document.
fn single_solid(document: &CadDocument) -> &acadrust::entities::Solid3D {
    let mut solids = document
        .model_space_entities()
        .filter_map(|entity| match entity {
            EntityType::Solid3D(solid) => Some(solid),
            _ => None,
        });
    let solid = solids.next().expect("one 3DSOLID record");
    assert!(solids.next().is_none(), "exactly one 3DSOLID record");
    solid
}

fn read_dxf(bytes: &[u8]) -> CadDocument {
    DxfReader::from_reader(Cursor::new(bytes.to_vec()))
        .expect("the written DXF parses")
        .read()
        .expect("the written DXF reads")
}

fn read_dwg(bytes: &[u8]) -> CadDocument {
    DwgReader::from_stream(Cursor::new(bytes.to_vec()))
        .read()
        .expect("acadrust reads back its own DWG output")
}

/// Assert one accepted combination end to end: the container header, the payload
/// header version, and the topology read back from the payload.
fn assert_combination(
    format: ExportFormat,
    acis_version: AcisVersion,
    dxf_target: DxfTarget,
    container: &str,
) {
    let authored = authored_box();
    let (bytes, report) = export_with(&box_database(), format, acis_version, dxf_target)
        .unwrap_or_else(|error| {
            panic!("{format:?} + {acis_version:?} + {dxf_target:?} must be accepted: {error:?}")
        });

    match format {
        ExportFormat::DxfText => {
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains(container),
                "{format:?} + {dxf_target:?} writes $ACADVER {container}: no {container} in the file"
            );
            let document = read_dxf(&bytes);
            assert_eq!(
                document.version,
                dxf_target.dxf_version(),
                "{format:?} + {dxf_target:?}: the read-back container version"
            );
            let solid = single_solid(&document);
            if dxf_target.stores_sab() {
                // R2013/R2018: a SAB/ASM blob in ACDSDATA, queued verbatim.
                let blob = acds_blob(&bytes);
                assert_eq!(
                    sab_version(&blob),
                    acis_version.number(),
                    "{format:?} + {dxf_target:?}: the ACDSDATA payload header version"
                );
                assert!(
                    solid.acis_data.is_binary && !solid.acis_data.sab_data.is_empty(),
                    "{format:?} + {dxf_target:?}: the reader attached the ACDSDATA blob"
                );
                let parsed = acadrust::SabReader::read(&blob)
                    .expect("the written SAB/ASM payload parses with our own SAB reader");
                assert_eq!(
                    parsed.header.version.sat_version_number(),
                    acis_version.number()
                );
                assert_eq!(
                    topology_counts(&parsed),
                    (1, 6, 12),
                    "{format:?} + {dxf_target:?}: shell/face/edge records of the box"
                );
                assert_eq!(
                    sab_flags(&blob) & 12,
                    if acis_version.number() >= 21800 {
                        12
                    } else {
                        0
                    },
                    "{format:?} + {dxf_target:?}: ShapeManager flags follow the payload version"
                );
                assert_brep_equivalent(&authored, &crate::sat_to_brep(&parsed));
            } else {
                // R2010: inline ACIS 7.0 SAT text, no ACDSDATA.
                //
                // The writer ciphers the group-1/3 lines (`AcisData::encode_sat`),
                // so the terminator is asserted on the decoded text — that is what
                // the reader sees, and it is still inline text, not a blob.
                let decoded = acadrust::entities::AcisData::decode_sat(
                    &dxf_pairs(&bytes)
                        .into_iter()
                        .filter(|(code, _)| *code == 1 || *code == 3)
                        .map(|(_, value)| value)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                assert!(
                    decoded.contains("End-of-ACIS-data"),
                    "R2010 carries the SAT text inline (after the DXF cipher decode)"
                );
                assert!(
                    !text.contains("ACDSDATA"),
                    "R2010 never writes an ACDSDATA SAB blob"
                );
                assert!(
                    !solid.acis_data.is_binary && !solid.acis_data.sat_data.is_empty(),
                    "the reader attached the inline SAT text"
                );
                let parsed = solid.acis_data.parse().expect("the inline SAT parses");
                assert_eq!(parsed.header.version.sat_version_number(), 700);
                assert_eq!(topology_counts(&parsed), (1, 6, 12));
                assert_brep_equivalent(&authored, &crate::sat_to_brep(&parsed));
            }
        }
        ExportFormat::Dwg => {
            assert!(
                bytes.starts_with(b"AC10"),
                "a DWG keeps its version signature"
            );
            let document = read_dwg(&bytes);
            assert_eq!(
                document.version,
                acadrust::DxfVersion::AC1032,
                "a DWG is AC1032 whatever the DXF target says"
            );
            assert!(
                document
                    .notifications
                    .iter()
                    .any(|notification| notification.message.starts_with("AcDs: attached")),
                "the reader attached the payload from the AcDs section"
            );
            let solid = single_solid(&document);
            assert!(
                solid.acis_data.is_binary && !solid.acis_data.sab_data.is_empty(),
                "the DWG payload is a binary SAB/ASM blob"
            );
            assert_eq!(
                sab_version(&solid.acis_data.sab_data),
                acis_version.number(),
                "the DWG AcDs payload header version"
            );
            let parsed = acadrust::SabReader::read(&solid.acis_data.sab_data)
                .expect("the AcDs blob parses with our own SAB reader");
            assert_eq!(topology_counts(&parsed), (1, 6, 12));
            assert_brep_equivalent(&authored, &crate::sat_to_brep(&parsed));
        }
    }

    // The report states the actual combination rather than a generic claim.
    let note = report
        .notes
        .iter()
        .find(|note| {
            note.code == "export.note.acis_sat_text_version"
                || note.code == "export.note.acis_sab_dxf"
                || note.code == "export.note.acis_sab_dwg"
        })
        .unwrap_or_else(|| panic!("no ACIS payload note in {:?}", report.notes));
    assert!(
        note.message.contains(acis_version.label()),
        "the note names the ACIS payload version: {}",
        note.message
    );
    assert!(
        note.message.contains(&acis_version.number().to_string()),
        "the note names the payload header version: {}",
        note.message
    );
    assert!(
        note.message.contains("no third-party CAD reader"),
        "the note states the evidence boundary: {}",
        note.message
    );
}

#[test]
fn every_allowed_combination_is_produced_and_reads_back() {
    // DXF R2010 + 7.0: inline SAT text (the historical default).
    assert_combination(
        ExportFormat::DxfText,
        AcisVersion::V7_0,
        DxfTarget::R2010,
        "AC1024",
    );
    // DXF R2013/R2018: SAB blob in ACDSDATA, both payload versions.
    for dxf_target in [DxfTarget::R2013, DxfTarget::R2018] {
        for acis_version in [AcisVersion::V7_0, AcisVersion::V21_0] {
            assert_combination(
                ExportFormat::DxfText,
                acis_version,
                dxf_target,
                dxf_target.dxf_version().as_str(),
            );
        }
    }
    // DWG: always AC1032/AcDs, both payload versions.
    for acis_version in [AcisVersion::V7_0, AcisVersion::V21_0] {
        assert_combination(ExportFormat::Dwg, acis_version, DxfTarget::R2018, "AC1032");
    }
}

#[test]
fn every_accepted_payload_keeps_the_same_topology_for_a_wedge() {
    // A different authored body, so the counts are not a coincidence of the box.
    let authored = wedge_brep(4.0, 2.0, 3.0);
    let database = db_with(vec![solid_body(authored.clone())]);
    let combinations = [
        (ExportFormat::DxfText, AcisVersion::V7_0, DxfTarget::R2010),
        (ExportFormat::DxfText, AcisVersion::V21_0, DxfTarget::R2013),
        (ExportFormat::DxfText, AcisVersion::V21_0, DxfTarget::R2018),
        (ExportFormat::Dwg, AcisVersion::V21_0, DxfTarget::R2018),
    ];
    for (format, acis_version, dxf_target) in combinations {
        let (bytes, _) = export_with(&database, format, acis_version, dxf_target)
            .unwrap_or_else(|error| panic!("{format:?}/{acis_version:?}: {error:?}"));
        let parsed = match format {
            ExportFormat::DxfText if !dxf_target.stores_sab() => read_dxf(&bytes)
                .model_space_entities()
                .find_map(|entity| match entity {
                    EntityType::Solid3D(solid) => solid.acis_data.parse(),
                    _ => None,
                })
                .expect("the inline SAT text parses"),
            ExportFormat::DxfText => {
                acadrust::SabReader::read(&acds_blob(&bytes)).expect("the ACDSDATA blob parses")
            }
            ExportFormat::Dwg => {
                let document = read_dwg(&bytes);
                let solid = single_solid(&document);
                acadrust::SabReader::read(&solid.acis_data.sab_data).expect("the AcDs blob parses")
            }
        };
        assert_eq!(
            topology_counts(&parsed),
            (1, 5, 9),
            "{format:?}/{acis_version:?}/{dxf_target:?}: wedge shell/face/edge records"
        );
        assert_brep_equivalent(&authored, &crate::sat_to_brep(&parsed));
    }
}

#[test]
fn an_asm_payload_in_a_pre_r2013_dxf_is_refused_by_name() {
    let error = export_with(
        &box_database(),
        ExportFormat::DxfText,
        AcisVersion::V21_0,
        DxfTarget::R2010,
    )
    .expect_err("an ASM 21800 payload cannot live in inline SAT text");
    match error {
        CadError::InvalidInput(message) => {
            assert!(
                message.starts_with("export.acis_asm_requires_r2013"),
                "the refusal leads with its stable code: {message}"
            );
            assert!(
                message.contains("21.0") && message.contains("21800"),
                "the refusal names the payload version: {message}"
            );
        }
        other => panic!("expected an explicit InvalidInput refusal, got {other:?}"),
    }
}

#[test]
fn a_dwg_with_any_other_dxf_target_is_refused_by_name() {
    for dxf_target in [DxfTarget::R2010, DxfTarget::R2013] {
        let error = export_with(
            &box_database(),
            ExportFormat::Dwg,
            AcisVersion::V7_0,
            dxf_target,
        )
        .expect_err("a DWG is fixed at AC1032, so the target is not silently ignored");
        match error {
            CadError::InvalidInput(message) => {
                assert!(
                    message.starts_with("export.dwg_version_fixed"),
                    "the refusal leads with its stable code: {message}"
                );
                assert!(
                    message.contains(dxf_target.label()),
                    "the refusal names the rejected target: {message}"
                );
            }
            other => panic!("expected an explicit InvalidInput refusal, got {other:?}"),
        }
    }
}

#[test]
fn the_defaults_are_the_preselected_combination() {
    // No selector present: ACIS 7.0 (700) in a R2010 (AC1024) DXF, exactly the
    // bytes the exporter produced before the selector existed.
    assert_eq!(AcisVersion::default(), AcisVersion::V7_0);
    assert_eq!(AcisVersion::default().number(), 700);
    assert_eq!(AcisVersion::default().label(), "7.0");
    assert_eq!(AcisVersion::V21_0.number(), 21800);
    assert_eq!(AcisVersion::V21_0.label(), "21.0");
    assert_eq!(DxfTarget::default(), DxfTarget::R2010);
    assert_eq!(
        DxfTarget::default().dxf_version(),
        acadrust::DxfVersion::AC1024
    );
    assert_eq!(DxfTarget::R2013.dxf_version(), acadrust::DxfVersion::AC1027);
    assert_eq!(DxfTarget::R2018.dxf_version(), acadrust::DxfVersion::AC1032);
    assert!(!DxfTarget::R2010.stores_sab());
    assert!(DxfTarget::R2013.stores_sab() && DxfTarget::R2018.stores_sab());
    assert_eq!(ExportOptions::default().acis_version, AcisVersion::V7_0);
    assert_eq!(ExportOptions::default().dxf_target, DxfTarget::R2010);

    // And the default export is still inline SAT text in AC1024.
    let (bytes, report) = export_db(&box_database());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("AC1024"));
    assert!(!text.contains("ACDSDATA"));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.code == "export.note.acis_sat_text_version"),
        "{:?}",
        report.notes
    );
}
