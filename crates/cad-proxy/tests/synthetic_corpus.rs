//! SYNTHETIC byte-level corpus for the proxy decoder.
//!
//! Every `.hex` file under `fixtures/proxy/` was hand-authored from the
//! documented record layout (acadrust 0.6.3 `ProxyGraphics`) by the committed
//! generator notes in `fixtures/proxy/README.md`. This is **not** vendor
//! evidence: these files were not produced by Tianzheng, TSSD, AutoCAD or any
//! real drawing. They exist to pin the fail-closed contract (framing bounds,
//! retained raw bytes, strict text terminator, mixed known/unknown sequences).
//!
//! Feed a new real sample here only after it is authorized and stored under
//! `fixtures/manifest`; synthetic data never proves format correctness by
//! itself.

use cad_domain::Completeness;
use cad_proxy::{ProxyPlayer, ProxySource};

fn source() -> ProxySource {
    ProxySource {
        handle: "1A".into(),
        class_name: "SYNTHETIC".into(),
        application: None,
        dwg_version: "SYNTHETIC".into(),
    }
}

fn bytes(hex: &str) -> Vec<u8> {
    let compact: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(compact.len() % 2, 0, "hex corpus file has an odd length");
    (0..compact.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&compact[i..i + 2], 16).unwrap())
        .collect()
}

const KNOWN_FILLOFF_TEXT: &str = include_str!("../../../fixtures/proxy/known_filloff_text.hex");
const UNKNOWN_OPCODE: &str = include_str!("../../../fixtures/proxy/unknown_opcode.hex");
const MIXED_KNOWN_UNKNOWN: &str = include_str!("../../../fixtures/proxy/mixed_known_unknown.hex");
const TRUNCATED_TEXT: &str = include_str!("../../../fixtures/proxy/truncated_text.hex");
const NO_TERMINATOR: &str = include_str!("../../../fixtures/proxy/no_terminator.hex");
const LYING_TOTAL_SIZE: &str = include_str!("../../../fixtures/proxy/lying_total_size.hex");

#[test]
fn known_filloff_and_text_decode_cleanly() {
    let out = ProxyPlayer::default()
        .replay(&source(), &bytes(KNOWN_FILLOFF_TEXT))
        .unwrap();
    assert_eq!(out.geometry.len(), 1);
    assert_eq!(out.completeness, Completeness::Complete);
    assert!(out.unsupported.is_empty());
}

#[test]
fn unknown_opcode_is_reported_with_raw_bytes() {
    let out = ProxyPlayer::default()
        .replay(&source(), &bytes(UNKNOWN_OPCODE))
        .unwrap();
    assert!(out.geometry.is_empty());
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert_eq!(out.unsupported.len(), 1);
    assert_eq!(out.unsupported[0].record_type, 999);
    assert_eq!(out.unsupported[0].data, vec![9, 9, 9, 9]);
}

#[test]
fn mixed_sequence_keeps_known_and_reports_unknown() {
    let out = ProxyPlayer::default()
        .replay(&source(), &bytes(MIXED_KNOWN_UNKNOWN))
        .unwrap();
    assert!(
        out.geometry.is_empty(),
        "only FillOff is geometry-free state"
    );
    assert!(matches!(out.completeness, Completeness::Missing(_)));
    assert_eq!(out.unsupported.len(), 1);
    assert_eq!(out.unsupported[0].record_type, 999);
    assert_eq!(out.unsupported[0].data, vec![9, 9, 9, 9]);
    assert!(out
        .diagnostics
        .iter()
        .any(|d| d.code == "proxy.unknown_opcode"));
}

#[test]
fn truncated_text_keeps_raw_bytes() {
    let out = ProxyPlayer::default()
        .replay(&source(), &bytes(TRUNCATED_TEXT))
        .unwrap();
    assert!(out.geometry.is_empty());
    assert_eq!(out.unsupported.len(), 1);
    assert_eq!(out.unsupported[0].record_type, 36);
    assert_eq!(out.unsupported[0].data, vec![0xAB; 8]);
}

#[test]
fn text_without_terminator_is_rejected() {
    let out = ProxyPlayer::default()
        .replay(&source(), &bytes(NO_TERMINATOR))
        .unwrap();
    assert!(out.geometry.is_empty());
    assert_eq!(out.unsupported.len(), 1);
    assert!(out.unsupported[0].reason.contains("terminator"));
}

#[test]
fn lying_total_size_is_rejected_at_framing() {
    assert!(ProxyPlayer::default()
        .replay(&source(), &bytes(LYING_TOTAL_SIZE))
        .is_err());
}
