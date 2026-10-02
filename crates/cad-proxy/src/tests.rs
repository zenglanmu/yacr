//! Unit tests.

use super::*;
use metafile::encode_for_test;

fn source() -> ProxySource {
    ProxySource {
        handle: "1A".into(),
        class_name: "TCH_WALL".into(),
        application: Some("Tianzheng".into()),
        dwg_version: "AC1027".into(),
    }
}

fn text_record(text: &str) -> Vec<u8> {
    let mut payload = vec![0u8; KnownOpcodeDecoder::UNICODE_TEXT_FIXED];
    // position (0,0,0), direction (1,0,0), height 2.5
    payload[72..80].copy_from_slice(&2.5f64.to_le_bytes());
    payload[48..56].copy_from_slice(&1.0f64.to_le_bytes());
    for u in text.encode_utf16() {
        payload.extend_from_slice(&u.to_le_bytes());
    }
    payload.extend_from_slice(&0u16.to_le_bytes());
    encode_for_test(36, &payload)
}

#[test]
fn known_records_replay_to_geometry() {
    let data = metafile::concat(&[encode_for_test(21, &[]), text_record("room")]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert_eq!(out.geometry.len(), 1);
    assert_eq!(out.completeness, Completeness::Complete);
    match &out.geometry[0] {
        SemanticGeometry::Text { text, .. } => assert_eq!(text, "room"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn all_geometry_records_are_preserved_not_just_the_first() {
    // Three UnicodeText records before any unknown opcode: the decoder must
    // return all three (audit B21: import used to keep only the first).
    let data = metafile::concat(&[text_record("a"), text_record("b"), text_record("c")]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert_eq!(out.geometry.len(), 3);
    assert_eq!(out.completeness, Completeness::Complete);
    let texts: Vec<&str> = out
        .geometry
        .iter()
        .map(|g| match g {
            SemanticGeometry::Text { text, .. } => text.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(texts, vec!["a", "b", "c"]);
}

#[test]
fn unknown_opcode_degrades_and_flags_incomplete() {
    let data = metafile::concat(&[encode_for_test(999, &[1u8, 2, 3, 4]), text_record("after")]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert!(out.geometry.is_empty());
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "proxy.unknown_opcode"));
}

#[test]
fn empty_cache_is_missing_not_faked() {
    let out = ProxyPlayer::default().replay(&source(), &[]).unwrap();
    assert!(out.geometry.is_empty());
    assert!(matches!(out.completeness, Completeness::Missing(_)));
}

#[test]
fn oversize_input_is_rejected() {
    let limits = DecodeLimits {
        max_bytes: 4,
        ..Default::default()
    };
    let player = ProxyPlayer::new(limits);
    assert!(player.replay(&source(), &[0u8; 32]).is_err());
}

/// A truncated type-36 payload must not yield guessed text; the record is
/// reported unsupported and its raw bytes retained.
#[test]
fn truncated_text_record_is_unsupported_with_raw_bytes() {
    // The framing is valid, but the payload is far shorter than 96 bytes.
    let body = encode_for_test(36, &[0xAB; 8]);
    let data = metafile::concat(&[body]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert!(out.geometry.is_empty());
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "proxy.corrupt" && d.message.contains("truncated")));
    assert_eq!(out.unsupported.len(), 1);
    assert_eq!(out.unsupported[0].record_type, 36);
    assert_eq!(out.unsupported[0].data, vec![0xAB; 8]);
}

/// A type-36 payload with no `0x0000` terminator is rejected rather than
/// silently accepting the whole tail (which acadrust types as `Unknown`).
#[test]
fn text_without_terminator_is_unsupported() {
    let mut payload = vec![0u8; KnownOpcodeDecoder::UNICODE_TEXT_FIXED];
    payload[72..80].copy_from_slice(&2.5f64.to_le_bytes());
    // "hi" with no terminator.
    for u in "hi".encode_utf16() {
        payload.extend_from_slice(&u.to_le_bytes());
    }
    let data = metafile::concat(&[encode_for_test(36, &payload)]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert!(out.geometry.is_empty());
    assert_eq!(out.unsupported.len(), 1);
    assert!(out.unsupported[0].reason.contains("terminator"));
}

/// An absurd vertex count from a decoder must be rejected before any of its
/// geometry is emitted (this previously appended the over-limit primitive).
#[test]
fn absurd_vertex_count_is_rejected_before_emitting() {
    struct BombDecoder;
    impl ProxyRecordDecoder for BombDecoder {
        fn registration(&self) -> Registration {
            Registration {
                type_key: "test.bomb".into(),
                version: 1,
                priority: 1,
                entity_types: vec!["ACAD_PROXY_ENTITY".into()],
                capabilities: vec!["bomb".into()],
            }
        }
        fn decode(
            &self,
            _record: &ProxyRecord<'_>,
            _state: &mut ReplayState,
            _limits: &DecodeLimits,
        ) -> CadResult<Vec<SemanticGeometry>> {
            Ok(vec![SemanticGeometry::Polyline {
                points: vec![Point3::default(); 1_000],
                bulges: Vec::new(),
                closed: false,
            }])
        }
    }
    let limits = DecodeLimits {
        max_vertices: 4,
        ..Default::default()
    };
    let player = ProxyPlayer::new(limits).with_decoder(Box::new(BombDecoder));
    let data = metafile::concat(&[encode_for_test(77, &[0u8; 4])]);
    let out = player.replay(&source(), &data).unwrap();
    assert!(
        out.geometry.is_empty(),
        "over-budget geometry must not be emitted: {:?}",
        out.geometry
    );
    // Nothing was emitted, so the result is Missing, never a faked Partial.
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "proxy.limit" && d.message.contains("vertex")));
}

/// A decoder chain deeper than `max_stack_depth` is rejected before it is
/// consulted, so a caller cannot build an unbounded dispatch bomb.
#[test]
fn recursion_bomb_decoder_chain_is_bounded() {
    struct NoopDecoder(u32);
    impl ProxyRecordDecoder for NoopDecoder {
        fn registration(&self) -> Registration {
            Registration {
                type_key: format!("test.noop.{}", self.0),
                version: 1,
                priority: 0,
                entity_types: vec!["ACAD_PROXY_ENTITY".into()],
                capabilities: vec!["noop".into()],
            }
        }
        fn decode(
            &self,
            _record: &ProxyRecord<'_>,
            _state: &mut ReplayState,
            _limits: &DecodeLimits,
        ) -> CadResult<Vec<SemanticGeometry>> {
            Err(CadError::Unsupported("noop".into()))
        }
    }
    let limits = DecodeLimits {
        max_stack_depth: 2,
        ..Default::default()
    };
    // One built-in decoder + three extras = 4 > 2.
    let mut player = ProxyPlayer::new(limits);
    for i in 0..3 {
        player = player.with_decoder(Box::new(NoopDecoder(i)));
    }
    let data = metafile::concat(&[encode_for_test(77, &[])]);
    let out = player.replay(&source(), &data).unwrap();
    assert!(out.geometry.is_empty());
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert!(out.unsupported[0].reason.contains("stack-depth"));
}

/// A mixed sequence: the known FillOff decodes, the unknown opcode is
/// reported, and the trailing text (after the unknown) is not claimed.
#[test]
fn mixed_known_then_unknown_keeps_known_and_reports_unknown() {
    let data = metafile::concat(&[
        encode_for_test(21, &[]),
        encode_for_test(999, &[9u8, 9, 9]),
        text_record("after"),
    ]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert_eq!(out.unsupported.len(), 1);
    assert_eq!(out.unsupported[0].record_type, 999);
    assert_eq!(out.unsupported[0].data, vec![9u8, 9, 9]);
    assert!(
        out.geometry.is_empty(),
        "the only decoded record is state-only FillOff"
    );
    // Known state decoded + unknown reported => partial/missing, never complete.
    assert_ne!(out.completeness, Completeness::Complete);
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "proxy.unknown_opcode"));
}

/// A known record before an unknown one still contributes geometry.
#[test]
fn known_geometry_before_unknown_is_preserved() {
    let data = metafile::concat(&[
        text_record("before"),
        encode_for_test(999, &[1u8, 2]),
        text_record("after"),
    ]);
    let out = ProxyPlayer::default().replay(&source(), &data).unwrap();
    assert_eq!(out.geometry.len(), 1);
    match &out.geometry[0] {
        SemanticGeometry::Text { text, .. } => assert_eq!(text, "before"),
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(out.completeness, Completeness::Partial(_)));
    assert_eq!(out.unsupported.len(), 1);
    assert!(!out.unsupported[0].data.is_empty());
}

#[test]
fn raw_dwg_bytes_are_not_treated_as_graphic_data() {
    let out = ProxyPlayer::default().inspect_raw_dwg(&[0xAB; 64]).unwrap();
    assert!(out.geometry.is_empty());
    assert_eq!(out.completeness, Completeness::Unverified);
}
