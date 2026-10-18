use super::*;
#[test]
fn mesh_draw_plan_makes_edge_overlay_optional() {
    assert_eq!(
        mesh_draw_plan(true, false),
        MeshDrawPlan {
            triangles: true,
            edge_overlay: false
        }
    );
    assert_eq!(
        mesh_draw_plan(true, true),
        MeshDrawPlan {
            triangles: true,
            edge_overlay: true
        }
    );
    assert_eq!(
        mesh_draw_plan(false, true),
        MeshDrawPlan {
            triangles: false,
            edge_overlay: false
        }
    );
}
