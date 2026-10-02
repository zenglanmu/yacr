//! Generate the synthetic ACIS fixtures under `fixtures/acis/`.
//!
//! Run from the workspace root:
//!
//! ```text
//! cargo run -p cad-import-acadrust --example generate_acis_fixtures -- fixtures/acis
//! ```
//!
//! The payloads are authored in this repository from acadrust's documented
//! primitive builders (and, for the holed box, from the same record layout by
//! hand). They are contract fixtures only: no vendor drawing is involved.

use acadrust::entities::acis::primitives::{build_box, build_cone, build_cylinder, build_sphere};
use acadrust::entities::acis::{SatDocument, SatPointer, SatToken, Sense, Sidedness};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/acis".to_string());
    fs::create_dir_all(&dir).expect("create fixtures dir");

    let write = |name: &str, doc: &SatDocument| {
        let path = PathBuf::from(&dir).join(name);
        fs::write(&path, doc.to_sat_string())
            .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        println!("wrote {}", path.display());
    };

    write("cube.sat", &build_box([0.0, 0.0, 0.0], 2.0, 2.0, 2.0));
    write("cylinder.sat", &build_cylinder([0.0, 0.0, 0.0], 1.0, 3.0));
    write("sphere.sat", &build_sphere([0.0, 0.0, 0.0], 2.0));
    write(
        "cone-unsupported.sat",
        &build_cone([0.0, 0.0, 0.0], 1.0, 2.0),
    );
    if let Some(doc) = box_with_square_hole() {
        write("box-with-square-hole.sat", &doc);
    } else {
        eprintln!("box-with-square-hole construction failed");
        std::process::exit(1);
    }
}

/// A 10x10x4 box with a 4x4 square through-hole, all faces planar.
///
/// Top and bottom are annuli (two loops each); the four outer and four inner
/// walls close the shell. Every undirected edge is shared by exactly two
/// coedges, so the tessellated result must weld closed.
fn box_with_square_hole() -> Option<SatDocument> {
    let vertices: Vec<[f64; 3]> = vec![
        // 0..4 outer bottom, 4..8 outer top
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
        [0.0, 0.0, 4.0],
        [10.0, 0.0, 4.0],
        [10.0, 10.0, 4.0],
        [0.0, 10.0, 4.0],
        // 8..12 hole bottom, 12..16 hole top
        [3.0, 3.0, 0.0],
        [7.0, 3.0, 0.0],
        [7.0, 7.0, 0.0],
        [3.0, 7.0, 0.0],
        [3.0, 3.0, 4.0],
        [7.0, 3.0, 4.0],
        [7.0, 7.0, 4.0],
        [3.0, 7.0, 4.0],
    ];
    use std::vec;
    #[rustfmt::skip]
    let faces: Vec<Vec<Vec<usize>>> = vec![
        // bottom (-Z): outer clockwise from +Z, hole counter-clockwise
        vec![
            vec![0, 3, 2, 1],
            vec![8, 9, 10, 11],
        ],
        // top (+Z)
        vec![
            vec![4, 5, 6, 7],
            vec![12, 15, 14, 13],
        ],
        // outer walls
        vec![vec![0, 1, 5, 4]],  // y = 0, -Y
        vec![vec![1, 2, 6, 5]],  // x = 10, +X
        vec![vec![2, 3, 7, 6]],  // y = 10, +Y
        vec![vec![3, 0, 4, 7]],  // x = 0, -X
        // inner walls (normals point into the hole)
        vec![vec![8, 11, 15, 12]], // x = 3, +X
        vec![vec![11, 10, 14, 15]],// y = 7, -Y
        vec![vec![10, 9, 13, 14]], // x = 7, -X
        vec![vec![9, 8, 12, 13]],  // y = 3, +Y
    ];
    build_looped_shell(&vertices, &faces)
}

struct EdgeInfo {
    edge: i32,
    start: usize,
    end: usize,
    coedges: Vec<i32>,
}

/// Build a closed shell whose faces may each have several loops.
///
/// This is the same record layout acadrust's `build_planar_body` uses, extended
/// to inner loops so a planar face can carry a hole.
fn build_looped_shell(vertices: &[[f64; 3]], faces: &[Vec<Vec<usize>>]) -> Option<SatDocument> {
    let mut sat = SatDocument::new_body();
    let body_idx = SatPointer::new(0);

    let mut vert_idx = Vec::with_capacity(vertices.len());
    for &[x, y, z] in vertices {
        let p = sat.add_point(x, y, z);
        vert_idx.push(sat.add_vertex(SatPointer::NULL, ptr(p)));
    }

    let mut edges: HashMap<(usize, usize), EdgeInfo> = HashMap::new();
    let mut edge_order = Vec::new();
    for face in faces {
        for loop_ in face {
            let n = loop_.len();
            if n < 3 {
                return None;
            }
            for i in 0..n {
                let a = loop_[i];
                let b = loop_[(i + 1) % n];
                if a == b || a >= vertices.len() || b >= vertices.len() {
                    return None;
                }
                let key = if a < b { (a, b) } else { (b, a) };
                if let std::collections::hash_map::Entry::Vacant(slot) = edges.entry(key) {
                    let sp = vertices[a];
                    let dir = vsub(vertices[b], sp);
                    let len = vnorm(dir);
                    if len < 1e-12 {
                        return None;
                    }
                    let ud = [dir[0] / len, dir[1] / len, dir[2] / len];
                    let crv = sat.add_straight_curve(sp, ud);
                    let e = sat.add_edge(
                        ptr(vert_idx[a]),
                        0.0,
                        ptr(vert_idx[b]),
                        len,
                        SatPointer::NULL,
                        ptr(crv),
                        Sense::Forward,
                    );
                    slot.insert(EdgeInfo {
                        edge: e,
                        start: a,
                        end: b,
                        coedges: Vec::new(),
                    });
                    edge_order.push(key);
                }
            }
        }
    }

    let mut surf_idx = Vec::with_capacity(faces.len());
    for face in faces {
        let (root, normal, u) = face_plane(vertices, &face[0])?;
        surf_idx.push(sat.add_plane_surface(root, normal, u));
    }

    let co_base = sat.records.len() as i32;
    let num_coedges: usize = faces.iter().flat_map(|f| f.iter()).map(|l| l.len()).sum();
    let num_loops: usize = faces.iter().map(|f| f.len()).sum();
    let loop_base = co_base + num_coedges as i32;
    let face_base = loop_base + num_loops as i32;
    let shell_idx = face_base + faces.len() as i32;
    let lump_idx = shell_idx + 1;

    let mut co_cursor = co_base;
    let mut loop_cursor = loop_base;
    let mut face_first_loop = Vec::with_capacity(faces.len());
    let mut loop_first_co = Vec::with_capacity(num_loops);
    let mut loop_face = Vec::with_capacity(num_loops);
    for (fi, face) in faces.iter().enumerate() {
        face_first_loop.push(loop_cursor);
        for loop_ in face {
            let start = co_cursor;
            loop_first_co.push(start);
            loop_face.push(face_base + fi as i32);
            let n = loop_.len() as i32;
            for i in 0..loop_.len() {
                let a = loop_[i];
                let b = loop_[(i + 1) % loop_.len()];
                let key = if a < b { (a, b) } else { (b, a) };
                let info = edges.get_mut(&key)?;
                let sense = if info.start == a && info.end == b {
                    Sense::Forward
                } else {
                    Sense::Reversed
                };
                let next = start + ((i as i32 + 1) % n);
                let prev = start + ((i as i32 + n - 1) % n);
                let idx = sat.add_coedge(
                    ptr(next),
                    ptr(prev),
                    SatPointer::NULL,
                    ptr(info.edge),
                    sense,
                    ptr(loop_cursor),
                );
                info.coedges.push(idx);
                co_cursor += 1;
            }
            loop_cursor += 1;
        }
    }

    for key in &edge_order {
        let info = &edges[key];
        if info.coedges.len() != 2 {
            return None;
        }
        set_partner(&mut sat, info.coedges[0], info.coedges[1]);
        set_partner(&mut sat, info.coedges[1], info.coedges[0]);
    }

    let mut loop_next = vec![SatPointer::NULL; num_loops];
    let mut li = 0usize;
    for face in faces {
        for k in 0..face.len() {
            if k + 1 < face.len() {
                loop_next[li] = ptr(loop_base + li as i32 + 1);
            }
            li += 1;
        }
    }
    for i in 0..num_loops {
        sat.add_loop(loop_next[i], ptr(loop_first_co[i]), ptr(loop_face[i]));
    }
    for fi in 0..faces.len() {
        let next_face = if fi + 1 < faces.len() {
            ptr(face_base + fi as i32 + 1)
        } else {
            SatPointer::NULL
        };
        sat.add_face(
            next_face,
            ptr(face_first_loop[fi]),
            ptr(shell_idx),
            ptr(surf_idx[fi]),
            Sense::Forward,
            Sidedness::Single,
        );
    }
    sat.add_shell(ptr(face_base), ptr(lump_idx));
    sat.add_lump(ptr(shell_idx), body_idx);
    if let Some(body_rec) = sat.record_mut(0) {
        body_rec.tokens[1] = SatToken::Pointer(ptr(lump_idx));
    }
    sat.complete_brep_links();
    if sat.validate().is_empty() {
        Some(sat)
    } else {
        None
    }
}

fn ptr(i: i32) -> SatPointer {
    SatPointer::new(i)
}

fn set_partner(sat: &mut SatDocument, coedge: i32, partner: i32) {
    if let Some(record) = sat.record_mut(coedge as usize) {
        record.tokens[3] = SatToken::Pointer(ptr(partner));
    }
}

fn face_plane(vertices: &[[f64; 3]], loop_: &[usize]) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
    if loop_.len() < 3 {
        return None;
    }
    let root = vertices[loop_[0]];
    let mut normal = [0.0; 3];
    for i in 0..loop_.len() {
        let a = vertices[loop_[i]];
        let b = vertices[loop_[(i + 1) % loop_.len()]];
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    if vnorm(normal) < 1e-12 {
        return None;
    }
    let normal = vscale(normal, 1.0 / vnorm(normal));
    let d = vsub(vertices[loop_[1]], root);
    if vnorm(d) < 1e-12 {
        return None;
    }
    Some((root, normal, vscale(d, 1.0 / vnorm(d))))
}

fn vsub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vnorm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn vscale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
