//! Stable persistent identity for ymodel documents (ADR-0037 D5).
//!
//! In memory the database keeps its historical per-family `u128` newtypes, so
//! `LayerId(1)` and `EntityId(1)` coexist and every existing "numeric equality"
//! invariant survives. On disk an identifier is a single `i64` composite:
//!
//! ```text
//! (kind << 45) | seq          kind: u8 (8 bits), seq: < 2^45
//! ```
//!
//! The whole value therefore stays under 2^53 and is exactly representable as
//! an IEEE-754 double, which is what JSON parsers and `jq` use. Identifiers
//! larger than 2^53 must be written as strings by whoever owns the field; this
//! module refuses to build one.
//!
//! `seq` is never reused. The high-water marks live in
//! [`super::DocumentHeader`], are persisted, and are restored verbatim on load
//! — reconstructing them from `max(id) + 1` would hand a deleted identifier to
//! a new object.

use crate::{
    BlockId, DimStyleId, EntityId, LayerId, LayoutId, LinetypeId, MLeaderStyleId, ObjectId,
    ScaleId, SpaceId, StyleId,
};

/// Bits reserved for the sequence part of a [`PersistentId`].
pub const SEQ_BITS: u32 = 45;

/// Largest sequence number a persisted identifier can carry.
pub const MAX_SEQ: u128 = (1 << SEQ_BITS) - 1;

/// Elements per `elements/<model>-<shard>.json` entry.
///
/// Part of the format contract: an element's shard depends only on its
/// sequence number, and sequences are never reused, so an element changes
/// shard only when it changes Model.
pub const SHARD_ELEMENTS: u128 = 2048;

/// Which identifier family a sequence number belongs to.
///
/// The numeric values are file-format constants: append new kinds, never
/// renumber or reuse an existing value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum IdKind {
    Entity = 1,
    Layer = 2,
    Block = 3,
    Layout = 4,
    Style = 5,
    DimStyle = 6,
    MLeaderStyle = 7,
    Linetype = 8,
    Scale = 9,
    /// A ymodel Model (model space, paper space, or a block model). Its
    /// sequence is derived from the owning layout/block, not allocated.
    Model = 10,
    /// The one SubCategory mirroring a DWG layer (D7).
    SubCategory = 11,
    Relationship = 12,
    Aspect = 13,
}

impl IdKind {
    /// All kinds, in format order. Used by registry checks and tests.
    pub const ALL: [IdKind; 13] = [
        IdKind::Entity,
        IdKind::Layer,
        IdKind::Block,
        IdKind::Layout,
        IdKind::Style,
        IdKind::DimStyle,
        IdKind::MLeaderStyle,
        IdKind::Linetype,
        IdKind::Scale,
        IdKind::Model,
        IdKind::SubCategory,
        IdKind::Relationship,
        IdKind::Aspect,
    ];

    /// Decode a persisted kind byte. `None` means "not this build's format" —
    /// callers must refuse the document, never guess a kind.
    pub const fn from_u8(value: u8) -> Option<IdKind> {
        match value {
            1 => Some(IdKind::Entity),
            2 => Some(IdKind::Layer),
            3 => Some(IdKind::Block),
            4 => Some(IdKind::Layout),
            5 => Some(IdKind::Style),
            6 => Some(IdKind::DimStyle),
            7 => Some(IdKind::MLeaderStyle),
            8 => Some(IdKind::Linetype),
            9 => Some(IdKind::Scale),
            10 => Some(IdKind::Model),
            11 => Some(IdKind::SubCategory),
            12 => Some(IdKind::Relationship),
            13 => Some(IdKind::Aspect),
            _ => None,
        }
    }
}

/// A persisted identifier: `(kind << 45) | seq`, always ≤ 2^53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersistentId(i64);

impl PersistentId {
    /// Build an identifier, refusing a sequence this format cannot store.
    pub const fn new(kind: IdKind, seq: u128) -> Option<PersistentId> {
        if seq > MAX_SEQ {
            return None;
        }
        // kind ≤ 13 and seq < 2^45, so the result is < 2^49: no overflow.
        Some(PersistentId(((kind as i64) << SEQ_BITS) | seq as i64))
    }

    /// Build an identifier from parts already known to be in range.
    ///
    /// # Panics
    ///
    /// Panics when `seq > MAX_SEQ`. Use it only for values that came from a
    /// validated document or from a sequence the allocator promised.
    pub const fn expect(kind: IdKind, seq: u128) -> PersistentId {
        match PersistentId::new(kind, seq) {
            Some(id) => id,
            None => panic!("persistent id sequence exceeds 45 bits"),
        }
    }

    /// Decode a raw on-disk identifier.
    ///
    /// A negative value, or a kind byte this build does not know, yields
    /// `None`: the document is from a different format generation.
    pub const fn from_raw(raw: i64) -> Option<PersistentId> {
        if raw < 0 || raw > MAX_ID {
            return None;
        }
        match PersistentId::kind_of(raw) {
            Some(_) => Some(PersistentId(raw)),
            None => None,
        }
    }

    /// The identifier family.
    pub const fn kind(self) -> IdKind {
        // Invariant: `PersistentId` is only ever built through `new`/`from_raw`.
        match IdKind::from_u8((self.0 >> SEQ_BITS) as u8) {
            Some(kind) => kind,
            None => unreachable!(),
        }
    }

    /// The sequence number inside its family.
    pub const fn seq(self) -> u128 {
        (self.0 & (MAX_SEQ as i64)) as u128
    }

    /// Raw composite value, as written to disk.
    pub const fn raw(self) -> i64 {
        self.0
    }

    const fn kind_of(raw: i64) -> Option<IdKind> {
        let kind = ((raw >> SEQ_BITS) & 0xff) as u8;
        IdKind::from_u8(kind)
    }
}

/// Largest representable persistent identifier (2^53 - 1).
pub const MAX_ID: i64 = (1 << 53) - 1;

/// An in-memory identifier newtype that has a stable persistent family.
///
/// Implemented for the existing `cad-domain` newtypes so the store can convert
/// without knowing their layout. `ObjectId` deliberately shares [`IdKind::Entity`]
/// with `EntityId`: they are one numeric space.
pub trait PersistentIdOf: Sized + Copy {
    /// The family this newtype belongs to.
    const KIND: IdKind;

    /// The sequence number this value carries inside its family.
    fn seq(self) -> u128;

    /// Rebuild the newtype from a sequence number.
    fn from_seq(seq: u128) -> Self;

    /// The persisted form.
    fn persistent(self) -> PersistentId {
        PersistentId::expect(Self::KIND, self.seq())
    }
}

macro_rules! persistent_ids {
    ($($name:ident => $kind:ident),* $(,)?) => {$(
        impl PersistentIdOf for $name {
            const KIND: IdKind = IdKind::$kind;

            fn seq(self) -> u128 {
                self.0
            }

            fn from_seq(seq: u128) -> Self {
                $name(seq)
            }
        }
    )*};
}

persistent_ids!(
    EntityId => Entity,
    ObjectId => Entity,
    LayerId => Layer,
    BlockId => Block,
    LayoutId => Layout,
    StyleId => Style,
    DimStyleId => DimStyle,
    MLeaderStyleId => MLeaderStyle,
    LinetypeId => Linetype,
    ScaleId => Scale,
);

/// A Model in the ymodel sense, derived from the in-memory space of an element.
///
/// Model identifiers are not allocated: a paper layout's model reuses the
/// layout's sequence under [`IdKind::Model`], a block's model reuses the
/// block's, and model space is sequence 0. Deleted layouts/blocks take their
/// model id with them and it is never handed out again, because the underlying
/// layout/block allocator never rewinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelKey {
    /// The single model space (`cad:ModelSpace`).
    ModelSpace,
    /// One paper layout (`cad:PaperSpace`).
    Paper(LayoutId),
    /// One block definition's contents (`cad:BlockModel`).
    Block(BlockId),
    /// The document-level `cad:DefinitionModel`: block definition elements,
    /// text/dimension/mleader styles, the layer category and its
    /// sub-categories.
    Definitions,
}

/// Sequence reserved for [`ModelKey::Definitions`].
///
/// It is the largest legal sequence, so it cannot collide with an allocated
/// layout or block id in any realistic document; a document that claims it for
/// a row is refused on load rather than silently sharing a model.
pub const DEFINITION_MODEL_SEQ: u128 = MAX_SEQ;

impl ModelKey {
    /// The Model this element space belongs to.
    pub const fn from_space(space: SpaceId) -> ModelKey {
        match space {
            SpaceId::Model => ModelKey::ModelSpace,
            SpaceId::Paper(layout) => ModelKey::Paper(layout),
            SpaceId::Block(block) => ModelKey::Block(block),
        }
    }

    /// The persisted Model identifier.
    pub const fn persistent(self) -> PersistentId {
        match self {
            ModelKey::ModelSpace => PersistentId::expect(IdKind::Model, 0),
            ModelKey::Paper(layout) => PersistentId::expect(IdKind::Model, layout.0),
            ModelKey::Block(block) => PersistentId::expect(IdKind::Model, block.0),
            ModelKey::Definitions => PersistentId::expect(IdKind::Model, DEFINITION_MODEL_SEQ),
        }
    }

    /// Rebuild a key from a persisted Model id plus the family its sequence
    /// belongs to.
    ///
    /// Decoding cannot be done from the id alone: paper sequences and block
    /// sequences are both just the owning layout/block id, so the family has to
    /// come from `shard_map` and the layout/block tables. Sequence 0 is always
    /// model space (`LayoutId(0)`/`BlockId(0)` are reserved and never
    /// allocated); a `Paper`/`Block` claim on sequence 0 is rejected here.
    pub const fn from_persistent(id: PersistentId, family: ModelFamily) -> Option<ModelKey> {
        if !matches!(id.kind(), IdKind::Model) {
            return None;
        }
        match (family, id.seq()) {
            (ModelFamily::ModelSpace, 0) => Some(ModelKey::ModelSpace),
            (ModelFamily::ModelSpace, _) => None,
            (ModelFamily::Paper, 0) => None,
            // `MAX_SEQ` is reserved for the definition model: a layout or block
            // row claiming it is rejected rather than sharing that model.
            (ModelFamily::Paper, DEFINITION_MODEL_SEQ) => None,
            (ModelFamily::Paper, seq) => Some(ModelKey::Paper(LayoutId(seq))),
            (ModelFamily::Block, 0) => None,
            (ModelFamily::Block, DEFINITION_MODEL_SEQ) => None,
            (ModelFamily::Block, seq) => Some(ModelKey::Block(BlockId(seq))),
            (ModelFamily::Definitions, DEFINITION_MODEL_SEQ) => Some(ModelKey::Definitions),
            (ModelFamily::Definitions, _) => None,
        }
    }
}

/// The table a Model sequence must be resolved against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelFamily {
    /// `cad:ModelSpace` (sequence 0 only).
    ModelSpace,
    /// `cad:PaperSpace`, resolved against the layouts table.
    Paper,
    /// `cad:BlockModel`, resolved against the blocks table.
    Block,
    /// The document-level definition model (reserved sequence only).
    Definitions,
}

/// A shard index inside one Model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Shard(pub u64);

impl Shard {
    /// Shard holding the given element sequence.
    pub const fn of(seq: u128) -> Shard {
        Shard((seq / SHARD_ELEMENTS) as u64)
    }

    /// First element sequence in this shard.
    pub const fn first_seq(self) -> u128 {
        self.0 as u128 * SHARD_ELEMENTS
    }
}

/// Shard holding the given element sequence.
pub const fn shard_of(seq: u128) -> Shard {
    Shard::of(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_values_are_file_format_constants() {
        // Pinned deliberately: renumbering silently reinterprets old files.
        let expected = [
            (IdKind::Entity, 1u8),
            (IdKind::Layer, 2),
            (IdKind::Block, 3),
            (IdKind::Layout, 4),
            (IdKind::Style, 5),
            (IdKind::DimStyle, 6),
            (IdKind::MLeaderStyle, 7),
            (IdKind::Linetype, 8),
            (IdKind::Scale, 9),
            (IdKind::Model, 10),
            (IdKind::SubCategory, 11),
            (IdKind::Relationship, 12),
            (IdKind::Aspect, 13),
        ];
        for (kind, byte) in expected {
            assert_eq!(kind as u8, byte, "{kind:?}");
            assert_eq!(IdKind::from_u8(byte), Some(kind));
        }
        assert_eq!(IdKind::from_u8(0), None);
        assert_eq!(IdKind::from_u8(14), None);
        assert_eq!(IdKind::ALL.len(), expected.len());
    }

    #[test]
    fn persistent_id_round_trips() {
        for kind in IdKind::ALL {
            for seq in [0u128, 1, 2047, 2048, MAX_SEQ] {
                let id = PersistentId::new(kind, seq).expect("in range");
                assert_eq!(id.kind(), kind);
                assert_eq!(id.seq(), seq);
                assert_eq!(PersistentId::from_raw(id.raw()), Some(id));
            }
        }
    }

    #[test]
    fn sequence_beyond_45_bits_is_rejected() {
        assert!(PersistentId::new(IdKind::Entity, MAX_SEQ).is_some());
        assert!(PersistentId::new(IdKind::Entity, MAX_SEQ + 1).is_none());
    }

    #[test]
    fn every_identifier_stays_inside_the_exact_json_range() {
        // 2^53 is where `f64`-backed JSON parsers start rounding integers.
        for kind in IdKind::ALL {
            let id = PersistentId::new(kind, MAX_SEQ).expect("in range");
            assert!(
                id.raw() <= MAX_ID,
                "{kind:?} at max sequence must stay ≤ 2^53-1, got {}",
                id.raw()
            );
        }
    }

    #[test]
    fn from_raw_rejects_negative_and_unknown_kind() {
        assert_eq!(PersistentId::from_raw(-1), None);
        // Kind 14 shifted into place is not a kind this build knows.
        let unknown = (14i64) << SEQ_BITS;
        assert_eq!(PersistentId::from_raw(unknown), None);
        // Above 2^53.
        assert_eq!(PersistentId::from_raw(MAX_ID + 1), None);
    }

    #[test]
    fn families_do_not_collide() {
        let entity = EntityId(1).persistent();
        let layer = LayerId(1).persistent();
        let block = BlockId(1).persistent();
        let layout = LayoutId(1).persistent();
        assert_ne!(entity, layer);
        assert_ne!(entity, block);
        assert_ne!(layer, layout);
        assert_eq!(
            entity,
            ObjectId(1).persistent(),
            "EntityId and ObjectId share a space"
        );
    }

    #[test]
    fn shards_follow_sequence_not_document_order() {
        assert_eq!(Shard::of(0), Shard(0));
        assert_eq!(Shard::of(2047), Shard(0));
        assert_eq!(Shard::of(2048), Shard(1));
        assert_eq!(Shard::of(4096), Shard(2));
        assert_eq!(Shard(2).first_seq(), 4096);
        assert_eq!(shard_of(4095), Shard(1));
    }

    #[test]
    fn model_keys_are_distinct_per_space() {
        let model_space = ModelKey::from_space(SpaceId::Model).persistent();
        let paper = ModelKey::from_space(SpaceId::Paper(LayoutId(1))).persistent();
        let block = ModelKey::from_space(SpaceId::Block(BlockId(1))).persistent();
        assert_eq!(ModelKey::from_space(SpaceId::Model), ModelKey::ModelSpace);
        assert_ne!(model_space, paper);
        assert_ne!(model_space, block);
        assert_eq!(
            ModelKey::from_persistent(model_space, ModelFamily::ModelSpace),
            Some(ModelKey::ModelSpace)
        );
        // Paper and block sequences overlap numerically on purpose: the family
        // has to come from the tables, never from the id alone.
        assert_eq!(
            ModelKey::from_persistent(paper, ModelFamily::Paper),
            Some(ModelKey::Paper(LayoutId(1)))
        );
        assert_eq!(
            ModelKey::from_persistent(paper, ModelFamily::Block),
            Some(ModelKey::Block(BlockId(1)))
        );
        assert_eq!(
            ModelKey::from_persistent(paper, ModelFamily::ModelSpace),
            None
        );
        assert_eq!(
            ModelKey::from_persistent(model_space, ModelFamily::Paper),
            None
        );
        assert_eq!(
            ModelKey::from_persistent(EntityId(1).persistent(), ModelFamily::ModelSpace),
            None
        );
    }

    #[test]
    fn definition_model_owns_a_reserved_sequence() {
        let definitions = ModelKey::Definitions.persistent();
        assert_eq!(definitions.kind(), IdKind::Model);
        assert_eq!(definitions.seq(), MAX_SEQ);
        assert_eq!(
            ModelKey::from_persistent(definitions, ModelFamily::Definitions),
            Some(ModelKey::Definitions)
        );
        // A layout or block row claiming the reserved sequence is refused
        // instead of silently sharing the definition model.
        assert_eq!(
            ModelKey::from_persistent(definitions, ModelFamily::Paper),
            None
        );
        assert_eq!(
            ModelKey::from_persistent(definitions, ModelFamily::Block),
            None
        );
        // And the definition model cannot be claimed by an ordinary sequence.
        assert_eq!(
            ModelKey::from_persistent(
                ModelKey::Paper(LayoutId(1)).persistent(),
                ModelFamily::Definitions
            ),
            None
        );
        assert_ne!(definitions, ModelKey::ModelSpace.persistent());
    }
}
