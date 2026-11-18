//! The nine persisted identifier high-water marks (ADR-0037 D5).
//!
//! [`DrawingDatabase`] keeps one monotonic `next_*_id` counter per allocated
//! family (entity, block, layer, layout, style, dimension style, multileader
//! style, linetype, scale). The counters are **document state**, not derived
//! values: they are written into `document.json` and restored verbatim on load.
//!
//! Rebuilding them from `max(row id) + 1` is forbidden. A document that once
//! held id `7`, deleted it and reopened would otherwise hand `7` to a new
//! object: the never-reuse rule is what keeps `SourceRef` maps and the
//! `seq / 2048` shard assignment stable, and a reused id would alias a stale
//! reference and can move a row into another shard.
//!
//! This module is the only place that maps the counters to
//! [`cad_domain::ymodel::IdCounters`] and back, so the builder's import path, a
//! row-by-row load and the scope's dirty-header check cannot disagree about
//! which fields make up the nine.

use cad_domain::ymodel::{IdCounters, COUNTER_CEILING};
use cad_domain::{CadError, CadResult};

use crate::drawing::DrawingDatabase;

impl DrawingDatabase {
    /// The nine counters exactly as the allocators hold them.
    ///
    /// No derived values: these are the high-water marks themselves, so writing
    /// them into `document.json` and seeding a later load with the same object
    /// is a fixed point.
    pub fn id_counters(&self) -> IdCounters {
        IdCounters {
            entity: self.next_entity_id,
            block: self.next_block_id,
            layer: self.next_layer_id,
            layout: self.next_layout_id,
            style: self.next_style_id,
            dim_style: self.next_dim_style_id,
            mleader_style: self.next_mleader_style_id,
            linetype: self.next_linetype_id,
            scale: self.next_scale_id,
        }
    }

    /// Adopt persisted counters on a database that was hydrated row by row.
    ///
    /// [`crate::DrawingDatabaseBuilder::seed_identifiers`] is the same operation
    /// for the builder path; this is the entry a load path that inserts rows
    /// directly uses. Both validate identically:
    ///
    /// * every counter is within `1 ..= MAX_SEQ + 1`;
    /// * every counter is at least `max(existing row id) + 1`, so the next mint
    ///   can never collide with a row already in the database.
    ///
    /// A counter below that floor is refused. It is **not** clamped: clamping
    /// would silently turn a corrupt or stale `document.json` into an allocator
    /// that reuses a live id.
    pub fn adopt_id_counters(&mut self, counters: &IdCounters) -> CadResult<()> {
        validate_against_rows(&self.id_counter_floor(), counters)?;
        self.next_entity_id = counters.entity;
        self.next_block_id = counters.block;
        self.next_layer_id = counters.layer;
        self.next_layout_id = counters.layout;
        self.next_style_id = counters.style;
        self.next_dim_style_id = counters.dim_style;
        self.next_mleader_style_id = counters.mleader_style;
        self.next_linetype_id = counters.linetype;
        self.next_scale_id = counters.scale;
        Ok(())
    }

    /// The lowest legal counter per family: `max(stored row id) + 1`.
    ///
    /// Entity ids also include block member lists defensively; those rows are
    /// normally present in [`Self::entities`] as well, but the import builder
    /// accepts a member list before every row is inserted, and the floor must
    /// never be lower than an id the document can reach.
    pub(crate) fn id_counter_floor(&self) -> IdCounters {
        let entity = floor_of(
            self.entities.keys().map(|id| id.0).chain(
                self.blocks
                    .values()
                    .flat_map(|b| b.entities.iter().map(|e| e.0)),
            ),
        );
        let block = floor_of(self.blocks.keys().map(|id| id.0));
        let layer = floor_of(self.layers.keys().map(|id| id.0));
        // `LayoutId(0)` is the reserved synthetic model row and is never
        // allocated, so the layout floor stays at 1 even when it is stored.
        let layout = floor_of(self.layouts.keys().map(|id| id.0)).max(1);
        let style = floor_of(self.styles.keys().map(|id| id.0));
        let dim_style = floor_of(self.dim_styles.keys().map(|id| id.0));
        let mleader_style = floor_of(self.mleader_styles.keys().map(|id| id.0));
        let linetype = floor_of(self.linetypes.keys().map(|id| id.0));
        let scale = floor_of(self.scales.keys().map(|id| id.0));
        IdCounters {
            entity,
            block,
            layer,
            layout,
            style,
            dim_style,
            mleader_style,
            linetype,
            scale,
        }
    }
}

/// `max(id) + 1` over one family, with `1` for an empty family.
fn floor_of(ids: impl Iterator<Item = u128>) -> u128 {
    ids.max().unwrap_or(0).saturating_add(1)
}

/// Refuse counters a document could not honestly carry.
///
/// `floor` is what [`DrawingDatabase::id_counter_floor`] computed: the lowest
/// counter that cannot reuse an existing id.
fn validate_against_rows(floor: &IdCounters, counters: &IdCounters) -> CadResult<()> {
    counters.validate().map_err(|error| {
        CadError::InvalidInput(format!(
            "identity.counter_out_of_range: {error}; a persisted counter must be in 1..={COUNTER_CEILING}"
        ))
    })?;
    for (name, value, minimum) in [
        ("entity", counters.entity, floor.entity),
        ("block", counters.block, floor.block),
        ("layer", counters.layer, floor.layer),
        ("layout", counters.layout, floor.layout),
        ("style", counters.style, floor.style),
        ("dim_style", counters.dim_style, floor.dim_style),
        ("mleader_style", counters.mleader_style, floor.mleader_style),
        ("linetype", counters.linetype, floor.linetype),
        ("scale", counters.scale, floor.scale),
    ] {
        if value < minimum {
            return Err(CadError::InvalidInput(format!(
                "identity.counter_below_high_water: the {name} counter is {value} but the \
                 document already uses ids up to {}; it must start at {minimum} or above so an \
                 existing or deleted id is never reused",
                minimum - 1
            )));
        }
    }
    Ok(())
}
