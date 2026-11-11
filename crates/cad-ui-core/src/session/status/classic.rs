//! The `blocks`, `groups` and `images` projections of the frozen wire.
//!
//! Three classic-UI panels read authoritative data only:
//!
//! * `blocks` is one row per drawing block definition (`BlockDefinition`), with
//!   the stored name/base-point metadata when the drawing has it. An unnamed
//!   definition keeps `name`/`basePoint` `None`; no zero base point is invented.
//! * `groups` is one row per **session-only** [`cad_app::EntityGroup`], with its
//!   captured member references. Groups are never DWG objects.
//! * `images` is one row per `SemanticGeometry::Image` entity in any space.
//!
//! Nothing here mutates the drawing, the session or a preference; every field is
//! copied from the authoritative record.

use cad_domain::{ImageClip, SpaceId};
use serde::Serialize;

/// One block definition (`cad_status.blocks[]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadBlockStatus {
    /// The stored table name, or `None` for a definition without metadata.
    pub name: Option<String>,
    /// The definition's real member count (`BlockDefinition::entities`).
    pub member_count: u64,
    /// The stored insertion base point, or `None` when the block has no metadata.
    pub base_point: Option<[f64; 3]>,
}

/// One member reference of a session group; ids stay strings so a `u128` entity
/// id never loses precision in JavaScript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadGroupMember {
    /// The entity id as a decimal string.
    pub entity_id: String,
    /// The nested `INSERT` chain, root first; empty for a top-level entity.
    pub instance_path: Vec<String>,
}

/// One session-only entity group (`cad_status.groups[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadGroupStatus {
    /// The stored group name.
    pub name: String,
    /// The captured member count (`members.len()`).
    pub member_count: u64,
    /// The captured references, in capture order and unique by `SelectionRef`.
    pub members: Vec<CadGroupMember>,
}

/// One image clip boundary summary (the full polygon is not needed to list it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadImageClipStatus {
    /// The stored vertex count (`2` is the rectangle form).
    pub vertex_count: u64,
    /// Whether the boundary keeps the inside (`true`) or the outside (`false`).
    pub inside: bool,
}

/// One raster `IMAGE` entity (`cad_status.images[]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CadImageStatus {
    /// The entity id as a decimal string.
    pub id: String,
    /// Stable space key: `model`, `paper` or `block`.
    pub space: String,
    /// The layout/block id for `paper`/`block`, `None` for model space.
    pub space_id: Option<u128>,
    /// The logical resource key stored on the entity, if any.
    pub file: Option<String>,
    /// Pixel size (`(width, height)`) as stored.
    pub pixels: [f64; 2],
    /// The entity's stored visibility flag.
    pub visible: bool,
    /// The stored clip boundary summary, or `None` when the image is unclipped.
    pub clip: Option<CadImageClipStatus>,
}

impl CadBlockStatus {
    /// Project one authoritative definition plus its optional metadata.
    pub(crate) fn from_definition(
        definition: &cad_db::BlockDefinition,
        metadata: Option<&cad_db::BlockMetadata>,
    ) -> Self {
        Self {
            name: metadata.map(|row| row.name.clone()),
            member_count: definition.entities.len() as u64,
            base_point: metadata.map(|row| [row.base_point.x, row.base_point.y, row.base_point.z]),
        }
    }
}

impl CadGroupStatus {
    /// Project one session group verbatim.
    pub(crate) fn from_group(group: &cad_app::EntityGroup) -> Self {
        Self {
            name: group.name.clone(),
            member_count: group.members.len() as u64,
            members: group
                .members
                .iter()
                .map(|reference| CadGroupMember {
                    entity_id: reference.entity.0.to_string(),
                    instance_path: reference
                        .instance
                        .0
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect(),
                })
                .collect(),
        }
    }
}

impl CadImageStatus {
    /// Project one image entity; the caller has already matched the geometry.
    pub(crate) fn from_entity(
        id: cad_domain::EntityId,
        space: &SpaceId,
        file: &Option<String>,
        pixels: [f64; 2],
        visible: bool,
        clip: Option<&ImageClip>,
    ) -> Self {
        let (space_key, space_id) = match space {
            SpaceId::Model => ("model", None),
            SpaceId::Paper(layout) => ("paper", Some(layout.0)),
            SpaceId::Block(block) => ("block", Some(block.0)),
        };
        Self {
            id: id.0.to_string(),
            space: space_key.to_string(),
            space_id,
            file: file.clone(),
            pixels,
            visible,
            clip: clip.map(|clip| CadImageClipStatus {
                vertex_count: clip.vertices.len() as u64,
                inside: clip.inside,
            }),
        }
    }
}
