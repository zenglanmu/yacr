//! Frame budget accounting and upload byte reporting.

use super::*;

#[test]
fn out_of_range_triangle_indices_are_dropped_not_uploaded() {
    let mut mesh = quad();
    mesh.triangles.push([0, 1, 9]); // index 9 does not exist
    let mut cache = SceneCache::default();
    let delta = cache.build(&mesh_representation(1, mesh), stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.indices, vec![[0, 1, 2], [0, 2, 3]]);
}

#[test]
fn frame_budget_reports_over_vertex_limit() {
    let budget = FrameBudget {
        max_vertices: 10,
        max_triangles: 10,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage::default();
    budget.charge(&mut usage, 6, 2).unwrap();
    let exceeded = budget.charge(&mut usage, 6, 0).unwrap_err();
    assert_eq!(exceeded.category, "vertices");
    assert_eq!(exceeded.requested, 12);
    assert_eq!(exceeded.limit, 10);
    // The rejected charge did not apply to the running usage.
    assert_eq!(usage.vertices, 6);
}

#[test]
fn frame_budget_reports_over_triangle_limit() {
    let budget = FrameBudget {
        max_vertices: 1000,
        max_triangles: 3,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage::default();
    budget.charge(&mut usage, 3, 2).unwrap();
    let exceeded = budget.charge(&mut usage, 3, 2).unwrap_err();
    assert_eq!(exceeded.category, "triangles");
    assert_eq!(exceeded.requested, 4);
    assert_eq!(exceeded.limit, 3);
    assert_eq!(usage.triangles, 2);
}

#[test]
fn frame_budget_reports_over_byte_limit() {
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: 100,
    };
    let mut usage = FrameUsage::default();
    budget.charge_bytes(&mut usage, 60).unwrap();
    let exceeded = budget.charge_bytes(&mut usage, 60).unwrap_err();
    assert_eq!(exceeded.category, "bytes");
    assert_eq!(exceeded.requested, 120);
    assert_eq!(exceeded.limit, 100);
    // The rejected charge did not apply.
    assert_eq!(usage.bytes, 60);
}

#[test]
fn frame_budget_rejects_counter_overflow_without_changing_usage() {
    let budget = FrameBudget {
        max_vertices: usize::MAX,
        max_triangles: usize::MAX,
        max_bytes: usize::MAX,
    };
    let mut usage = FrameUsage {
        vertices: usize::MAX,
        triangles: usize::MAX,
        bytes: usize::MAX,
    };
    let before = usage;
    assert_eq!(
        budget.charge(&mut usage, 1, 0),
        Err(BudgetExceeded {
            category: "vertices",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    assert_eq!(
        budget.charge(&mut usage, 0, 1),
        Err(BudgetExceeded {
            category: "triangles",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    assert_eq!(
        budget.charge_bytes(&mut usage, 1),
        Err(BudgetExceeded {
            category: "bytes",
            requested: usize::MAX,
            limit: usize::MAX,
        })
    );
    assert_eq!(usage, before);
    budget.charge(&mut usage, 0, 0).unwrap();
    budget.charge_bytes(&mut usage, 0).unwrap();
    assert_eq!(usage, before);
}

#[test]
fn upload_bytes_of_counts_every_uploaded_buffer() {
    // A mesh batch uploads positions, normals, per-vertex colours, `[u32; 3]`
    // triangle indices and a `u32` edge index per de-duplicated edge.
    let mut cache = SceneCache::default();
    let delta = cache
        .build(&mesh_representation(1, quad()), stamp())
        .unwrap();
    let batch = &delta.added[0];
    // 4 positions + 4 normals = 8 vertex vectors * 12 = 96.
    // 4 per-vertex colours * 12 = 48 (a mesh always binds the colour attribute).
    // 2 triangles * 12 = 24 index bytes.
    // A quad triangulated as [0,1,2],[0,2,3] has 5 distinct undirected edges
    // (the diagonal is shared) => 10 `u32` edge indices = 40 bytes.
    // `edges` (the position list the scene carries) is separate and counted too.
    assert_eq!(batch.vertex_bytes(), (4 + 4 + 12) * 12);
    assert_eq!(edge_index_count(&batch.indices), 10);
    let expected = batch.vertex_bytes() + 4 * 12 + 2 * 12 + 10 * 4;
    assert_eq!(batch.upload_size_bytes(), expected);
}

#[test]
fn upload_bytes_of_line_batch_has_no_index_bytes() {
    let mut cache = SceneCache::default();
    let rep = line_representation(
        1,
        vec![
            Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            Point3 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        ],
    );
    let delta = cache.build(&rep, stamp()).unwrap();
    let batch = &delta.added[0];
    assert_eq!(batch.upload_size_bytes(), 2 * 12);
}
