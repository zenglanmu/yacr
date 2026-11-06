//! Measurement, not a gate: what one `SceneDelta` clone costs.
//!
//! The incremental append path used to materialise the resident base + the new
//! entity by cloning the whole `SceneDelta` (`render_scene/mod.rs`, before the
//! prefix/suffix split). That clone is the reason an "incremental" insert on a
//! large drawing still cost O(scene). This test exists so the numbers behind
//! that decision can be reproduced instead of being folklore.
//!
//! `#[ignore]`d on purpose: it is a timing measurement, it never asserts a
//! threshold, and CI must not fail because a machine is busy. Run it manually:
//!
//! ```text
//! cargo test -p cad-app --release --locked --test scene_delta_clone_cost \
//!     -- --ignored --nocapture
//! ```
//!
//! Recorded 2026-11-06 on this workstation (release): 44,591 batches (the
//! `complex-test.dwg` scale) -> 3.95 ms per clone; 200,000 batches -> 41.0 ms per
//! clone. NOT a GPU measurement and NOT portable to other machines.

use std::time::Instant;

use cad_domain::{DocumentId, EntityId, InstancePath, Point3, SelectionRef, TaskStamp};
use cad_scene::{LinetypePattern, RenderBatch, RenderTopology, SceneDelta, DEFAULT_BATCH_COLOR};

fn batch(index: usize) -> RenderBatch {
    let x = index as f32 * 0.25;
    RenderBatch {
        conics: Vec::new(),
        local_origin: Point3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        topology: RenderTopology::Lines,
        vertices: vec![
            [x, 0.0, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x + 0.1, 0.1, 0.0],
            [x, 0.2, 0.0],
        ],
        normals: Vec::new(),
        colors: Vec::new(),
        indices: Vec::new(),
        edges: Vec::new(),
        mirrored: false,
        alpha: 1.0,
        color: DEFAULT_BATCH_COLOR,
        color_unresolved: true,
        lineweight: 0.0,
        lineweight_unresolved: true,
        linetype: LinetypePattern::continuous(),
        linetype_unresolved: true,
        sources: vec![SelectionRef {
            document: DocumentId(1),
            entity: EntityId(index as u128),
            instance: InstancePath::default(),
            sub_element: None,
        }],
        draw_order: index as i64,
    }
}

fn scene(n: usize) -> SceneDelta {
    SceneDelta {
        stamp: TaskStamp::new(DocumentId(1), 0),
        added: (0..n).map(batch).collect(),
        images: Vec::new(),
        masks: Vec::new(),
    }
}

#[test]
#[ignore = "timing measurement, not a gate; run manually with --ignored --nocapture"]
fn scene_delta_clone_cost() {
    for n in [44_591usize, 200_000] {
        let delta = scene(n);
        let _ = delta.clone();
        let mut best = f64::INFINITY;
        for _ in 0..5 {
            let start = Instant::now();
            let copy = delta.clone();
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(copy.added.len(), delta.added.len());
        }
        println!("clone added={n} batches: best {best:.3} ms");
    }
}
