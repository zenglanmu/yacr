//! Output accumulator and the block/clip bookkeeping shared by every writer.
//!
//! Split out of [`super::entity`] so that module stays under the 1000-line
//! limit. Nothing here decides *what* geometry becomes; it collects the output
//! entities, the explicit conversion/drop entries, and the block-reference state
//! the BLOCKS/INSERT and XCLIP writers coordinate through.

use std::collections::{BTreeMap, BTreeSet};

use acadrust::entities::EntityType;
use cad_db::DbEntity;
use cad_domain::*;

use super::entity::EntityContext;
use super::report::{ExportCounts, ExportDiagnostic, MappingOutcome};

/// A block definition written into the output, as one `BLOCK_RECORD`.
#[derive(Debug, Clone)]
pub(crate) struct WrittenBlock {
    /// The table name the record was written with.
    pub name: String,
    /// `true` when a dynamic-block visibility descriptor reduced the member set.
    pub visibility_applied: bool,
}

/// Mutable output accumulated while mapping.
pub(crate) struct ExportAccumulator {
    pub entities: Vec<EntityType>,
    pub counts: ExportCounts,
    pub entries: Vec<ExportDiagnostic>,
    pub notes: Vec<ExportDiagnostic>,
    /// Block definitions reached by an INSERT expansion (the explode fallback).
    pub expanded_blocks: BTreeSet<BlockId>,
    /// Block definitions written as real `BLOCK_RECORD`s, by id.
    pub written_blocks: BTreeMap<BlockId, WrittenBlock>,
    /// Blocks a reference reached that could not be written, with the reason.
    pub refused_blocks: BTreeMap<BlockId, String>,
    /// Number of block references written as real `INSERT` records.
    pub written_inserts: usize,
    /// Monotonic counter for generated `*D...` dimension content-block names.
    pub dim_block_counter: usize,
    /// While true, [`ExportAccumulator::record`] drops its entry: used to write
    /// a dimension's content-block members without counting each one as a
    /// separate source geometry (the dimension itself is one outcome).
    pub suppress_records: bool,
    /// MULTILEADER style records written so far, keyed by the lower-cased style
    /// name: one `MLEADERSTYLE` object per distinct name, first snapshot wins
    /// (ADR 0019 decision D).
    pub mleader_styles: BTreeMap<String, (String, MLeaderStyleValues)>,
    /// `true` once any `3DSOLID` record was queued — including one emitted as a
    /// block member and then drained into a `BLOCK_RECORD`.
    ///
    /// The DXF writer serialises a `3DSOLID` payload inline as SAT text only
    /// before AC1027 (see `export::export`), so the format decision must see a
    /// solid wherever it was emitted; reading this flag on `entities` alone would
    /// miss a body written inside a block definition.
    pub written_solid3d: bool,
    /// Source objects whose `XCLIP` boundary was already reported. A clipped
    /// reference reaches the entity mapper once per array cell and once per
    /// block-member walk with the same source object, so the loss is recorded
    /// once.
    pub reported_clips: BTreeSet<ObjectId>,
}

impl ExportAccumulator {
    pub(crate) fn new() -> Self {
        ExportAccumulator {
            entities: Vec::new(),
            counts: ExportCounts::default(),
            entries: Vec::new(),
            notes: Vec::new(),
            expanded_blocks: BTreeSet::new(),
            written_blocks: BTreeMap::new(),
            refused_blocks: BTreeMap::new(),
            written_inserts: 0,
            dim_block_counter: 0,
            suppress_records: false,
            mleader_styles: BTreeMap::new(),
            written_solid3d: false,
            reported_clips: BTreeSet::new(),
        }
    }

    pub(crate) fn record(&mut self, object: Option<ObjectId>, kind: &str, outcome: MappingOutcome) {
        if self.suppress_records {
            return;
        }
        match outcome {
            MappingOutcome::Exact => self.counts.exact += 1,
            MappingOutcome::Converted(reasons) => {
                self.counts.converted += 1;
                for reason in reasons {
                    self.entries.push(ExportDiagnostic {
                        object,
                        code: format!("export.converted.{kind}"),
                        message: reason,
                    });
                }
            }
            MappingOutcome::Dropped(reasons) => {
                self.counts.dropped += 1;
                for reason in reasons {
                    self.entries.push(ExportDiagnostic {
                        object,
                        code: format!("export.dropped.{kind}"),
                        message: reason,
                    });
                }
            }
        }
    }

    /// Record a drop under an explicit stable code.
    ///
    /// Most drops use the derived `export.dropped.<kind>` spelling. This entry
    /// point exists for losses whose code is itself the contract and names the
    /// missing feature (`export.xref_content_dropped`,
    /// `export.xref_definition_not_written`, `export.xclip_clip_dropped`) rather
    /// than only the entity kind. It still counts as a drop, so the report can
    /// never stay `Complete`.
    pub(crate) fn record_dropped_code(
        &mut self,
        object: Option<ObjectId>,
        code: &str,
        reasons: Vec<String>,
    ) {
        if self.suppress_records {
            return;
        }
        self.counts.dropped += 1;
        for reason in reasons {
            self.entries.push(ExportDiagnostic {
                object,
                code: code.to_string(),
                message: reason,
            });
        }
    }

    /// Record a conversion under an explicit stable code.
    ///
    /// Most conversions use the derived `export.converted.<kind>` spelling. This
    /// entry point exists for outcomes whose code is itself the contract and
    /// names the missing feature (`acis.curved_face_not_written`,
    /// `acis.multi_shell_not_written`, ...) rather than only the entity kind. It
    /// still counts as one converted geometry, so the report can never stay
    /// `Complete`.
    pub(crate) fn record_converted_code(
        &mut self,
        object: Option<ObjectId>,
        code: &str,
        reasons: Vec<String>,
    ) {
        if self.suppress_records {
            return;
        }
        self.counts.converted += 1;
        for reason in reasons {
            self.entries.push(ExportDiagnostic {
                object,
                code: code.to_string(),
                message: reason,
            });
        }
    }

    /// Record a source `XCLIP` boundary the output cannot carry.
    ///
    /// A boundary is a **display attribute** of a block reference. It is written
    /// as the reference's `ACAD_FILTER`/`SPATIAL_FILTER` chain whenever the
    /// reference itself is written as one real `INSERT`
    /// (see [`super::insert`]); this entry is for the cases where it is not.
    /// `reason` names which: the reference is exploded (with the exploded
    /// reference's own reason), the reference is refused outright, or the stored
    /// clip is an inverted region the format has no field for. The loss is named
    /// instead of silently exporting the block content unclipped. Deduped per
    /// source object: an array reference and every block-member walk reach the
    /// entity mapper with the same object identity.
    pub(super) fn record_clip_dropped(
        &mut self,
        entity: &DbEntity,
        ctx: &EntityContext,
        reason: &str,
    ) {
        if self.suppress_records {
            return;
        }
        let Some(clip) = ctx.database.entity_render_attributes(entity.id).clip else {
            return;
        };
        if !self.reported_clips.insert(entity.object.id) {
            return;
        }
        self.record_dropped_code(
            Some(entity.object.id),
            "export.xclip_clip_dropped",
            vec![format!(
                "the block reference's XCLIP boundary ({} point(s), switched {}) is not written: \
                 {reason}",
                clip.vertices.len(),
                if clip.visible { "on" } else { "off" }
            )],
        );
    }

    pub(super) fn push(&mut self, entity: EntityType, ctx: &EntityContext) -> CadResult<()> {
        if self.entities.len() >= ctx.limits.max_output_entities {
            return Err(CadError::InvalidInput(format!(
                "export exceeds the {} entity output limit",
                ctx.limits.max_output_entities
            )));
        }
        if matches!(entity, EntityType::Solid3D(_)) {
            self.written_solid3d = true;
        }
        self.entities.push(entity);
        Ok(())
    }
}
