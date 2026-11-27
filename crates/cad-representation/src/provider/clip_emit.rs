//! Region clipping of already-transformed display primitives.
//!
//! Split out of `expand.rs` for the file-size ceiling (`check-file-size.py`).
//! [`emit_clipped`] is the one place that cuts a transformed primitive to a
//! containing INSERT's `XCLIP` region; the two `clip_*_region` helpers apply the
//! shared clippers piece by piece.

use crate::layout::{clip_mesh, visible_image, ImageRegion, WorldRectBoundary};
use crate::*;

use super::expand::report_xclip;

/// Emit one already-transformed primitive through the block-clip region.
///
/// Meshes (including filled meshes and the wide-polyline stroke band), images
/// and — through their own arms — lines and conics are cut exactly. A region
/// with several disjoint pieces (a nested intersection) is clipped piece by
/// piece and each surviving piece is emitted as its own fragment, so the union
/// of the fragments is exactly `primitive ∩ clip`.
///
/// A primitive with no exact clip path is **kept** and reported, never dropped
/// or silently drawn as clipped; a clipper that refuses makes the whole
/// primitive stay unclipped (never a partial cut) and report its stable code.
///
/// The clip **depth** band is a separate filter applied before this one (see
/// `primitive_depth_span`): it drops whole primitives outside the band and keeps
/// a straddling primitive whole, reported by name.
pub(super) fn emit_clipped(
    entity: &DbEntity,
    primitive: DisplayPrimitive,
    region: &WorldRectBoundary,
    emit: &dyn Fn(DisplayPrimitive, &mut DisplayRepresentation),
    out: &mut DisplayRepresentation,
) {
    match primitive {
        DisplayPrimitive::Mesh(mesh) => match clip_mesh_region(&mesh, region) {
            Ok(pieces) => {
                for piece in pieces {
                    emit(DisplayPrimitive::Mesh(Arc::new(piece)), out);
                }
            }
            Err(_) => {
                report_xclip(
                    out,
                    entity,
                    "representation.xclip_mesh_unclippable",
                    "the mesh cannot be clipped exactly (missing index or non-finite vertex), \
                     so it is kept unclipped",
                );
                emit(DisplayPrimitive::Mesh(mesh), out);
            }
        },
        DisplayPrimitive::MeshWithoutEdgeOverlay(mesh) => match clip_mesh_region(&mesh, region) {
            Ok(pieces) => {
                for piece in pieces {
                    emit(
                        DisplayPrimitive::MeshWithoutEdgeOverlay(Arc::new(piece)),
                        out,
                    );
                }
            }
            Err(_) => {
                report_xclip(
                    out,
                    entity,
                    "representation.xclip_mesh_unclippable",
                    "the mesh cannot be clipped exactly (missing index or non-finite \
                     vertex), so it is kept unclipped",
                );
                emit(DisplayPrimitive::MeshWithoutEdgeOverlay(mesh), out);
            }
        },
        // A body's stored edge set has no exact per-piece region cut: the
        // clipper cuts the triangles only. A body whose every mesh vertex and
        // stored edge endpoint is inside the region cannot be cut, so it keeps
        // the variant (and its stored edges) with no diagnostic. Otherwise each
        // surviving piece is emitted as a plain `Mesh`, whose documented overlay
        // is derived from the clipped triangles, and the loss is reported once —
        // never stored edges *and* a derived overlay on the same piece.
        DisplayPrimitive::MeshWithStoredEdges { mesh, edges, .. } => {
            if region.holds_all_points(mesh.vertices.iter().chain(edges.iter())) {
                emit(
                    DisplayPrimitive::MeshWithStoredEdges {
                        mesh,
                        edges,
                        derive_when_empty: true,
                    },
                    out,
                );
                return;
            }
            match clip_mesh_region(&mesh, region) {
                Ok(pieces) => {
                    if !pieces.is_empty() {
                        report_xclip(
                            out,
                            entity,
                            "representation.xclip_body_edges_derived",
                            "stored body edges cannot be clipped exactly per region piece, so the \
                             clipped mesh draws its triangle-derived wireframe",
                        );
                    }
                    for piece in pieces {
                        emit(DisplayPrimitive::Mesh(Arc::new(piece)), out);
                    }
                }
                Err(_) => {
                    report_xclip(
                        out,
                        entity,
                        "representation.xclip_mesh_unclippable",
                        "the mesh cannot be clipped exactly (missing index or non-finite vertex), \
                         so it is kept unclipped",
                    );
                    emit(
                        DisplayPrimitive::MeshWithStoredEdges {
                            mesh,
                            edges,
                            derive_when_empty: true,
                        },
                        out,
                    );
                }
            }
        }
        DisplayPrimitive::Image {
            resource,
            transform,
            clip,
            adjust,
        } => {
            // The shared image path cuts the quad (or the entity's own clip
            // ring) against each region piece and hands back convex pieces with
            // interpolated texture coordinates — one fragment each, over the
            // same resource, so a concave clip is the union of its pieces.
            let mut whole: Option<Option<Arc<[ImageVertex]>>> = None;
            let mut pieces: Vec<Vec<ImageVertex>> = Vec::new();
            let mut refusal = None;
            for piece in region.pieces() {
                match visible_image(&transform, clip.as_deref(), piece) {
                    Ok(ImageRegion::Empty) => {}
                    Ok(ImageRegion::Whole { entity }) => {
                        // The whole quad survives this piece. Pieces are
                        // disjoint, so at most one can hold it; keeping the
                        // first is the exact answer.
                        whole.get_or_insert(entity);
                    }
                    Ok(ImageRegion::Pieces(parts)) => pieces.extend(parts),
                    Err(clip_refusal) => {
                        refusal = Some(clip_refusal);
                        break;
                    }
                }
            }
            if let Some(clip_refusal) = refusal {
                report_xclip(
                    out,
                    entity,
                    "representation.xclip_image_unclippable",
                    &clip_refusal.message,
                );
                emit(
                    DisplayPrimitive::Image {
                        resource,
                        transform,
                        clip,
                        adjust,
                    },
                    out,
                );
            } else if let Some(entity_clip) = whole {
                emit(
                    DisplayPrimitive::Image {
                        resource,
                        transform,
                        clip: entity_clip,
                        adjust,
                    },
                    out,
                );
            } else {
                for piece in pieces {
                    emit(
                        DisplayPrimitive::Image {
                            resource: resource.clone(),
                            transform,
                            clip: Some(Arc::from(piece.into_boxed_slice())),
                            adjust,
                        },
                        out,
                    );
                }
            }
        }
        // Text needs a shaped font outline and a mask is an erase region with
        // its own fill rule; neither has an exact region cut here.
        other => {
            let name = match &other {
                DisplayPrimitive::Text { .. } => "text",
                DisplayPrimitive::Mask { .. } => "mask",
                _ => "primitive",
            };
            report_xclip(
                out,
                entity,
                "representation.xclip_primitive_unsupported",
                &format!(
                    "{name} geometry has no exact block-clip path, so it is kept and drawn \
                     unclipped"
                ),
            );
            emit(other, out);
        }
    }
}

/// Clip one mesh against every piece of `region`.
///
/// `Ok(pieces)` holds the surviving meshes (empty when the whole mesh is outside,
/// an exact omission); `Err` is the shared clipper's refusal and means the
/// caller must keep the source mesh untouched.
fn clip_mesh_region(mesh: &Mesh, region: &WorldRectBoundary) -> Result<Vec<Mesh>, ClipRefusal> {
    let mut pieces = Vec::new();
    for piece in region.pieces() {
        if let Some(clipped) = clip_mesh(mesh, piece)? {
            pieces.push(clipped);
        }
    }
    Ok(pieces)
}
