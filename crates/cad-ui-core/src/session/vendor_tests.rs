use super::*;
use cad_app::{VendorRecord, VendorSource};

fn record(handle: u64) -> VendorRecord {
    VendorRecord {
        class_name: "TCH_WALL".to_string(),
        handle,
        dwg_version: 1027,
        source: VendorSource::UnknownEntity,
        main: vec![0x06, 0b1010_0000],
        main_bits: 10,
        text: vec![0b1100_0000],
        text_bits: 2,
        handle_stream: vec![0b0100_0000],
        handle_bits: 2,
        main_skip_bits: 3,
        text_skip_bits: 0,
        handle_skip_bits: 0,
        references: vec![0x2c],
    }
}

fn result_frame(
    token: u64,
    generation: u64,
    status: u32,
    applied: u32,
    diagnostics: &[&str],
) -> Vec<u8> {
    result_frame_with(token, generation, status, applied, diagnostics, &[])
}

fn result_frame_with(
    token: u64,
    generation: u64,
    status: u32,
    applied: u32,
    diagnostics: &[&str],
    entities: &[(u64, Vec<VendorPrimitive>)],
) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(&RESULT_MAGIC.to_le_bytes());
    frame.extend_from_slice(&VENDOR_ABI_VERSION.to_le_bytes());
    frame.extend_from_slice(&token.to_le_bytes());
    frame.extend_from_slice(&generation.to_le_bytes());
    frame.extend_from_slice(&status.to_le_bytes());
    frame.extend_from_slice(&applied.to_le_bytes());
    frame.extend_from_slice(&(diagnostics.len() as u32).to_le_bytes());
    for diagnostic in diagnostics {
        frame.extend_from_slice(&(diagnostic.len() as u16).to_le_bytes());
        frame.extend_from_slice(diagnostic.as_bytes());
    }
    frame.extend_from_slice(&(entities.len() as u32).to_le_bytes());
    for (handle, primitives) in entities {
        frame.extend_from_slice(&handle.to_le_bytes());
        frame.extend_from_slice(&(primitives.len() as u32).to_le_bytes());
        for primitive in primitives {
            match primitive {
                VendorPrimitive::Lines(points) => {
                    frame.push(0);
                    frame.extend_from_slice(&(points.len() as u32).to_le_bytes());
                    for point in points {
                        for value in [point.x, point.y, point.z] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
                VendorPrimitive::Segments(points) => {
                    frame.push(1);
                    frame.extend_from_slice(&(points.len() as u32).to_le_bytes());
                    for point in points {
                        for value in [point.x, point.y, point.z] {
                            frame.extend_from_slice(&value.to_le_bytes());
                        }
                    }
                }
                VendorPrimitive::Arc {
                    center,
                    radius,
                    start,
                    sweep,
                } => {
                    frame.push(2);
                    for value in [center.x, center.y, center.z, *radius, *start, *sweep] {
                        frame.extend_from_slice(&value.to_le_bytes());
                    }
                }
                VendorPrimitive::Circle { center, radius } => {
                    frame.push(3);
                    for value in [center.x, center.y, center.z, *radius] {
                        frame.extend_from_slice(&value.to_le_bytes());
                    }
                }
                VendorPrimitive::Text(text) => {
                    frame.push(4);
                    for value in [
                        text.origin.x,
                        text.origin.y,
                        text.origin.z,
                        text.height,
                        text.rotation,
                        text.wrap_width.unwrap_or(0.0),
                    ] {
                        frame.extend_from_slice(&value.to_le_bytes());
                    }
                    frame.push(text.attachment);
                    let bytes = text.text.as_bytes();
                    frame.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                    frame.extend_from_slice(bytes);
                }
                VendorPrimitive::Block {
                    block,
                    origin,
                    scale,
                    rotation,
                } => {
                    frame.push(5);
                    frame.extend_from_slice(&block.to_le_bytes());
                    for value in [
                        origin.x, origin.y, origin.z, scale.x, scale.y, scale.z, *rotation,
                    ] {
                        frame.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
    }
    frame
}

fn line(handle: u64) -> (u64, Vec<VendorPrimitive>) {
    (
        handle,
        vec![VendorPrimitive::Lines(vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 10.0,
                y: 0.0,
                z: 0.0,
            },
        ])],
    )
}

#[test]
fn records_frame_is_the_frozen_v1_layout() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(0x2c)]).expect("stage");
    let frame = store.records_frame(token).expect("active token");
    assert_eq!(&frame[0..4], &RECORDS_MAGIC.to_le_bytes());
    assert_eq!(&frame[4..8], &VENDOR_ABI_VERSION.to_le_bytes());
    assert_eq!(&frame[8..16], &token.to_le_bytes());
    assert_eq!(&frame[16..24], &generation.to_le_bytes());
    assert_eq!(&frame[24..28], &1u32.to_le_bytes());
    let class_len = u16::from_le_bytes([frame[28], frame[29]]) as usize;
    assert_eq!(&frame[30..30 + class_len], b"TCH_WALL");
    assert_eq!(
        store.records_frame(token + 1),
        Err("vendor.stale_token".into())
    );
}

#[test]
fn staging_without_enable_is_refused() {
    let mut store = VendorStore::default();
    assert_eq!(
        store.stage(vec![record(1)]),
        Err("vendor.disabled".to_string())
    );
}

#[test]
fn a_primitive_result_settles_once_and_reports() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    assert_eq!(
        store.announce_pending().map(|(t, _, n)| (t, n)),
        Some((token, 1))
    );
    assert_eq!(store.announce_pending(), None, "announced once");
    let entity = line(1);
    let frame = result_frame_with(token, generation, 0, 1, &[], &[entity]);
    let settlement = store
        .settle_bytes_full(token, generation, &frame)
        .expect("settles");
    assert!(settlement.ack.accepted);
    assert_eq!(settlement.ack.applied, 1);
    assert_eq!(settlement.primitives.len(), 1);
    assert_eq!(settlement.primitives[0].primitives.len(), 1);
    assert_eq!(store.take_unreported().map(|(t, _)| t), Some(token));
    assert_eq!(store.take_unreported(), None, "reported only once");
    let again = store
        .settle_bytes(token, generation, &frame)
        .expect("same ack");
    assert_eq!(again, settlement.ack);
}

#[test]
fn a_block_insert_primitive_keeps_its_handle_scale_and_rotation() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let frame = result_frame_with(
        token,
        generation,
        0,
        1,
        &[],
        &[(
            1,
            vec![VendorPrimitive::Block {
                block: 0x3dd82,
                origin: Point3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                },
                scale: Point3 {
                    x: 2.0,
                    y: -0.5,
                    z: 2.0,
                },
                rotation: 0.25,
            }],
        )],
    );
    let settlement = store
        .settle_bytes_full(token, generation, &frame)
        .expect("settles");
    assert!(settlement.ack.accepted);
    match &settlement.primitives[0].primitives[0] {
        VendorPrimitive::Block {
            block,
            scale,
            rotation,
            ..
        } => {
            assert_eq!(*block, 0x3dd82);
            assert_eq!(scale.y, -0.5, "the mirror scale is preserved");
            assert_eq!(*rotation, 0.25);
        }
        other => panic!("expected a block primitive, got {other:?}"),
    }
}

#[test]
fn a_block_insert_with_a_zero_scale_is_rejected() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let frame = result_frame_with(
        token,
        generation,
        0,
        1,
        &[],
        &[(
            1,
            vec![VendorPrimitive::Block {
                block: 0x3dd82,
                origin: Point3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                scale: Point3 {
                    x: 0.0,
                    y: 1.0,
                    z: 1.0,
                },
                rotation: 0.0,
            }],
        )],
    );
    assert_eq!(
        store.settle_bytes_full(token, generation, &frame),
        Err("vendor.invalid_block".to_string())
    );
}

#[test]
fn stale_tokens_and_generations_are_rejected() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let frame = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
    assert_eq!(
        store.settle_bytes(token + 1, generation, &frame),
        Err("vendor.stale_token".into())
    );
    assert_eq!(
        store.settle_bytes(token, generation + 1, &frame),
        Err("vendor.stale_generation".into())
    );
    store.clear();
    assert_eq!(
        store.settle_bytes(token, generation, &frame),
        Err("vendor.no_pending_pass".into())
    );
}

#[test]
fn corrupt_frames_are_rejected_in_order() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let mut bad_magic = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
    bad_magic[0] ^= 0xff;
    assert_eq!(
        store.settle_bytes(token, generation, &bad_magic),
        Err("vendor.bad_magic".into())
    );
    let mut bad_abi = result_frame_with(token, generation, 0, 1, &[], &[line(1)]);
    bad_abi[4..8].copy_from_slice(&(VENDOR_ABI_VERSION + 1).to_le_bytes());
    assert!(store
        .settle_bytes(token, generation, &bad_abi)
        .unwrap_err()
        .starts_with("vendor.unsupported_abi"));
    let mut wrong_token = result_frame_with(token + 1, generation, 0, 1, &[], &[line(1)]);
    wrong_token[8..16].copy_from_slice(&(token + 1).to_le_bytes());
    assert_eq!(
        store.settle_bytes(token, generation, &wrong_token),
        Err("vendor.frame_token_mismatch".into())
    );
    assert_eq!(
        store.settle_bytes(
            token,
            generation,
            &result_frame(token, generation, 7, 0, &[])
        ),
        Err("vendor.invalid_status".into())
    );
    assert_eq!(
        store.settle_bytes(
            token,
            generation,
            &result_frame(token, generation, 0, 1, &[])
        ),
        Err("vendor.applied_mismatch".into())
    );
}

#[test]
#[ignore]
fn parses_a_real_plugin_result() {
    let Ok(path) = std::env::var("YACR_VENDOR_RESULT") else {
        return;
    };
    let bytes = std::fs::read(path).expect("read result frame");
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let settlement = store
        .settle_bytes_full(token, generation, &bytes)
        .expect("real plugin frame settles");
    let total = settlement
        .primitives
        .iter()
        .map(|entity| entity.primitives.len())
        .sum::<usize>();
    eprintln!(
        "real plugin result: applied={} entities={} primitives={} diagnostics={}",
        settlement.ack.applied,
        settlement.primitives.len(),
        total,
        settlement.ack.diagnostics.len()
    );
    assert_eq!(settlement.ack.applied as usize, total);
    assert!(total > 0);
}

#[test]
fn a_record_pass_may_not_settle_as_an_empty_success() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    assert_eq!(
        store.settle_bytes(
            token,
            generation,
            &result_frame(token, generation, 0, 0, &[])
        ),
        Err("vendor.empty_success".into())
    );
}

#[test]
fn diagnostics_are_bounded_and_eof_is_exact() {
    let mut store = VendorStore::default();
    store.enable();
    let (token, generation) = store.stage(vec![record(1)]).expect("stage");
    let many: Vec<String> = (0..=MAX_DIAGNOSTICS).map(|i| i.to_string()).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(
        store.settle_bytes(
            token,
            generation,
            &result_frame(token, generation, 0, 0, &refs)
        ),
        Err("vendor.too_many_diagnostics".into())
    );

    let mut trailing = result_frame(token, generation, 0, 0, &[]);
    trailing.push(0);
    assert_eq!(
        store.settle_bytes(token, generation, &trailing),
        Err("vendor.trailing_bytes".into())
    );

    let unsupported = result_frame(token, generation, 1, 0, &["vendor.unsupported_layout"]);
    let ack = store
        .settle_bytes(token, generation, &unsupported)
        .expect("settles as unsupported");
    assert!(!ack.accepted);
    assert_eq!(ack.diagnostics, ["vendor.unsupported_layout"]);
}

#[test]
fn base64_round_trips_and_rejects_garbage() {
    let bytes = result_frame(1, 2, 0, 0, &["ok"]);
    let encoded = encode_base64(&bytes);
    assert_eq!(decode_base64(&encoded).expect("decodes"), bytes);
    assert!(decode_base64("!!!!").is_err());
    assert!(decode_base64("QQ===").is_err());
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let mut buffer = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            buffer |= (*byte as u32) << (16 - index * 8);
        }
        let chars = match chunk.len() {
            1 => 2,
            2 => 3,
            _ => 4,
        };
        for index in 0..4 {
            if index < chars {
                let shift = 18 - index * 6;
                output.push(TABLE[((buffer >> shift) & 0x3f) as usize] as char);
            } else {
                output.push('=');
            }
        }
    }
    output
}
