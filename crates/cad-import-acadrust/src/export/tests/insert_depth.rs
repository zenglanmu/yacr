//! `XCLIP` **clip depth** round-trip through the DXF `SPATIAL_FILTER` chain.
//!
//! `BlockClip` stores the front/back distances as a block-local z band; the
//! exporter writes them as WCS lengths in groups `40`/`41` with the enabled
//! flags `72`/`73`, and the importer reads the band back. These tests pin both
//! flags, both distances, the one-sided case (a flag/distance pair per side, no
//! distance without its flag) and the z-scale conversion; the boundary itself is
//! covered by `insert_blocks.rs`.
//!
//! Writer + our own reader agreement is **internal consistency**, not a
//! third-party compatibility claim.

use super::*;

use super::insert_blocks::{
    assert_vertices_close, clipped_database, clipped_database_with_scale, model_clip,
};

/// The group pairs of the single `SPATIAL_FILTER` record (its `0` marker to the
/// next record start), so a test can read the depth flags/distances exactly.
fn spatial_filter_pairs(bytes: &[u8]) -> Vec<(i32, String)> {
    let pairs = dxf_pairs(bytes);
    let start = pairs
        .iter()
        .position(|(code, value)| *code == 0 && value == "SPATIAL_FILTER")
        .expect("a SPATIAL_FILTER record");
    pairs[start + 1..]
        .iter()
        .take_while(|(code, _)| *code != 0)
        .cloned()
        .collect()
}

fn pair_number(pairs: &[(i32, String)], index: usize) -> f64 {
    pairs[index].1.parse::<f64>().unwrap_or_else(|_| {
        panic!(
            "group {} is not a number: {:?}",
            pairs[index].0, pairs[index]
        )
    })
}

#[test]
fn a_clip_depth_band_round_trips_both_distances_and_both_flags() {
    let stored = BlockClip::new(vec![[-2.0, -1.0], [3.0, 4.0]], true, true)
        .with_depth(Some(-1.5), Some(2.5))
        .unwrap();
    let database = clipped_database(stored.clone());
    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.xclip_clip_dropped"
                && entry.code != "export.xclip_depth_dropped"),
        "boundary and depth are written: {:?}",
        report.entries
    );

    // The object carries the plane normal the distances are measured along, and
    // each distance follows its own enabled flag (72/40 then 73/41).
    let pairs = spatial_filter_pairs(&bytes);
    let index = |code: i32| {
        pairs
            .iter()
            .position(|(pair_code, _)| *pair_code == code)
            .unwrap_or_else(|| panic!("group {code} missing: {pairs:?}"))
    };
    assert_eq!(pair_number(&pairs, index(210)), 0.0);
    assert_eq!(pair_number(&pairs, index(220)), 0.0);
    assert_eq!(pair_number(&pairs, index(230)), 1.0);
    let front_flag = index(72);
    assert_eq!(pairs[front_flag].1, "1", "the front flag is set: {pairs:?}");
    assert_eq!(pairs[front_flag + 1].0, 40);
    assert!((pair_number(&pairs, front_flag + 1) - (-1.5)).abs() < 1e-9);
    let back_flag = index(73);
    assert!(back_flag > front_flag, "{pairs:?}");
    assert_eq!(pairs[back_flag].1, "1", "the back flag is set");
    assert_eq!(pairs[back_flag + 1].0, 41);
    assert!((pair_number(&pairs, back_flag + 1) - 2.5).abs() < 1e-9);

    // ...and the reader recovers both distances and the boundary.
    let reimported = import_bytes(&bytes);
    let restored = model_clip(&reimported).expect("the boundary comes back");
    assert_vertices_close(&restored.vertices, &stored.vertices);
    assert_eq!(restored.front_clip, stored.front_clip);
    assert_eq!(restored.back_clip, stored.back_clip);
    assert!(restored.is_active());
}

#[test]
fn a_one_sided_clip_depth_writes_only_its_own_flag_and_distance() {
    let stored = BlockClip::new(vec![[-2.0, -1.0], [3.0, 4.0]], true, true)
        .with_depth(None, Some(2.0))
        .unwrap();
    let database = clipped_database(stored);
    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.xclip_depth_dropped"),
        "{:?}",
        report.entries
    );
    let pairs = spatial_filter_pairs(&bytes);
    let front = pairs
        .iter()
        .find(|(code, _)| *code == 72)
        .expect("the front flag is written as 0, not omitted");
    assert_eq!(front.1, "0", "no front plane: {pairs:?}");
    assert!(
        !pairs.iter().any(|(code, value)| *code == 40
            && value.parse::<f64>().is_ok_and(|v| (v - 2.0).abs() < 1e-9)),
        "a back-only band writes no front distance: {pairs:?}"
    );
    assert_eq!(
        pairs
            .iter()
            .find(|(code, _)| *code == 73)
            .map(|p| p.1.as_str()),
        Some("1")
    );

    let reimported = import_bytes(&bytes);
    let restored = model_clip(&reimported).expect("the boundary comes back");
    assert_eq!(restored.front_clip, None, "the absent side stays unbounded");
    assert_eq!(restored.back_clip, Some(2.0));
}

#[test]
fn a_clip_depth_band_round_trips_through_a_scaled_reference() {
    // The filter's distances are WCS lengths and the block-local band is the
    // stored unit, so a z-scaled reference converts on write and back on read.
    let stored = BlockClip::new(vec![[0.0, 0.0], [4.0, 4.0]], true, true)
        .with_depth(Some(1.0), Some(3.0))
        .unwrap();
    let database = clipped_database_with_scale(stored, [1.0, 1.0, 2.0]);
    let (bytes, report) = export_db(&database);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.code != "export.xclip_depth_dropped"),
        "{:?}",
        report.entries
    );
    let pairs = spatial_filter_pairs(&bytes);
    let back_flag = pairs.iter().position(|(code, _)| *code == 73).unwrap();
    assert!(
        (pairs[back_flag + 1].1.parse::<f64>().unwrap() - 6.0).abs() < 1e-9,
        "block-local z 3 on a x2 z scale is a WCS distance of 6: {pairs:?}"
    );
    let restored = model_clip(&import_bytes(&bytes)).expect("the boundary comes back");
    assert_eq!(restored.front_clip, Some(1.0));
    assert_eq!(restored.back_clip, Some(3.0));
}

#[test]
fn a_depth_band_with_no_world_length_is_named_not_invented() {
    // Defensive path: `decompose` refuses a reference without a positive finite z
    // scale before a clip is written, so this pins the message the writer would
    // report if that gate ever moved. The boundary is still writeable; only the
    // depth band has no WCS length, and it must say so by name.
    let clip = BlockClip::new(vec![[0.0, 0.0], [1.0, 1.0]], true, true)
        .with_depth(Some(1.0), Some(2.0))
        .unwrap();
    let mut matrix = Transform3::identity().matrix;
    matrix[2][2] = 0.0;
    let error = super::super::insert::depth_world(&Transform3 { matrix }, &clip)
        .expect_err("a zero z scale cannot carry the distances");
    assert!(
        error.contains("groups 40/41") && error.contains("1..2"),
        "the reason names the format field and the band: {error}"
    );
}
