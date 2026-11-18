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
    /// `cad:ModelSpace`. Its sequence is always 0.
    ModelSpace = 10,
    /// The one SubCategory mirroring a DWG layer (D7).
    SubCategory = 11,
    Relationship = 12,
    Aspect = 13,
    /// `cad:PaperSpace`; the sequence is the owning layout id (never 0).
    PaperModel = 14,
    /// `cad:BlockModel`; the sequence is the owning block id (never 0).
    BlockModel = 15,
    /// The document-level `cad:DefinitionModel`. Its sequence is always 0.
    DefinitionModel = 16,
}

impl IdKind {
    /// All kinds, in format order. Used by registry checks and tests.
    pub const ALL: [IdKind; 16] = [
        IdKind::Entity,
        IdKind::Layer,
        IdKind::Block,
        IdKind::Layout,
        IdKind::Style,
        IdKind::DimStyle,
        IdKind::MLeaderStyle,
        IdKind::Linetype,
        IdKind::Scale,
        IdKind::ModelSpace,
        IdKind::SubCategory,
        IdKind::Relationship,
        IdKind::Aspect,
        IdKind::PaperModel,
        IdKind::BlockModel,
        IdKind::DefinitionModel,
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
            10 => Some(IdKind::ModelSpace),
            11 => Some(IdKind::SubCategory),
            12 => Some(IdKind::Relationship),
            13 => Some(IdKind::Aspect),
            14 => Some(IdKind::PaperModel),
            15 => Some(IdKind::BlockModel),
            16 => Some(IdKind::DefinitionModel),
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
/// Model identifiers are not allocated: each kind of model has its own
/// [`IdKind`], so paper and block models that share a numeric sequence are
/// still distinct identifiers. A paper layout's model reuses the layout's
/// sequence under [`IdKind::PaperModel`], a block's model reuses the block's
/// under [`IdKind::BlockModel`], and model space / the definition model are
/// sequence 0 under their own kinds. Deleted layouts/blocks take their model id
/// with them and it is never handed out again, because the underlying
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
    ///
    /// # Panics
    ///
    /// Paper and block models encode their owner's sequence, and sequence 0 is
    /// reserved for [`ModelKey::ModelSpace`]. The allocators never issue it, so
    /// only a malformed row — `Paper(LayoutId(0))` or `Block(BlockId(0))` —
    /// reaches the panic (a debug assertion when they are compiled out); such a
    /// row is refused on load by [`ModelKey::from_persistent`], never built from
    /// a document.
    pub const fn persistent(self) -> PersistentId {
        match self {
            ModelKey::ModelSpace => PersistentId::expect(IdKind::ModelSpace, 0),
            ModelKey::Paper(layout) => {
                debug_assert!(
                    layout.0 != 0,
                    "layout sequence 0 is reserved for model space"
                );
                PersistentId::expect(IdKind::PaperModel, layout.0)
            }
            ModelKey::Block(block) => {
                debug_assert!(block.0 != 0, "block sequence 0 is reserved for model space");
                PersistentId::expect(IdKind::BlockModel, block.0)
            }
            ModelKey::Definitions => PersistentId::expect(IdKind::DefinitionModel, 0),
        }
    }

    /// Rebuild a key from a persisted Model id.
    ///
    /// The kind carries the family, so no side table is needed: `ModelSpace`
    /// and `DefinitionModel` are sequence 0, `PaperModel` and `BlockModel` are
    /// any non-zero sequence. Sequence 0 is reserved because the layout and
    /// block allocators never issue it, so a `PaperModel`/`BlockModel` id with
    /// sequence 0 is not a model this format can mean.
    pub const fn from_persistent(id: PersistentId) -> Option<ModelKey> {
        match (id.kind(), id.seq()) {
            (IdKind::ModelSpace, 0) => Some(ModelKey::ModelSpace),
            (IdKind::ModelSpace, _) => None,
            (IdKind::PaperModel, 0) => None,
            (IdKind::PaperModel, seq) => Some(ModelKey::Paper(LayoutId(seq))),
            (IdKind::BlockModel, 0) => None,
            (IdKind::BlockModel, seq) => Some(ModelKey::Block(BlockId(seq))),
            (IdKind::DefinitionModel, 0) => Some(ModelKey::Definitions),
            (IdKind::DefinitionModel, _) => None,
            _ => None,
        }
    }
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
            (IdKind::ModelSpace, 10),
            (IdKind::SubCategory, 11),
            (IdKind::Relationship, 12),
            (IdKind::Aspect, 13),
            (IdKind::PaperModel, 14),
            (IdKind::BlockModel, 15),
            (IdKind::DefinitionModel, 16),
        ];
        for (kind, byte) in expected {
            assert_eq!(kind as u8, byte, "{kind:?}");
            assert_eq!(IdKind::from_u8(byte), Some(kind));
        }
        assert_eq!(IdKind::from_u8(0), None);
        assert_eq!(IdKind::from_u8(17), None);
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
        // Kind 17 shifted into place is not a kind this build knows.
        let unknown = (17i64) << SEQ_BITS;
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
        // A paper layout and a block with the same sequence are different
        // models: the kind, not the sequence, carries the family.
        assert_eq!(paper.kind(), IdKind::PaperModel);
        assert_eq!(block.kind(), IdKind::BlockModel);
        assert_ne!(
            ModelKey::Paper(LayoutId(1)).persistent(),
            ModelKey::Block(BlockId(1)).persistent()
        );
        assert_eq!(
            ModelKey::from_persistent(model_space),
            Some(ModelKey::ModelSpace)
        );
        assert_eq!(
            ModelKey::from_persistent(paper),
            Some(ModelKey::Paper(LayoutId(1)))
        );
        assert_eq!(
            ModelKey::from_persistent(block),
            Some(ModelKey::Block(BlockId(1)))
        );
        assert_eq!(ModelKey::from_persistent(EntityId(1).persistent()), None);
    }

    #[test]
    fn definition_model_uses_sequence_zero_under_its_own_kind() {
        let definitions = ModelKey::Definitions.persistent();
        assert_eq!(definitions.kind(), IdKind::DefinitionModel);
        assert_eq!(definitions.seq(), 0);
        assert_eq!(
            ModelKey::from_persistent(definitions),
            Some(ModelKey::Definitions)
        );
        assert_ne!(definitions, ModelKey::ModelSpace.persistent());
        // An ordinary paper sequence cannot claim the definition model, and a
        // definition-model id with a non-zero sequence is not a model at all.
        assert_eq!(
            ModelKey::from_persistent(ModelKey::Paper(LayoutId(1)).persistent()),
            Some(ModelKey::Paper(LayoutId(1)))
        );
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::DefinitionModel, 1)),
            None
        );
    }

    #[test]
    fn from_persistent_rejects_reserved_and_mismatched_sequences() {
        // Model space and the definition model are sequence 0 only.
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::ModelSpace, 1)),
            None
        );
        // Paper and block models never carry sequence 0: it belongs to model
        // space, and no allocator issues it.
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::PaperModel, 0)),
            None
        );
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::BlockModel, 0)),
            None
        );
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::PaperModel, 1)),
            Some(ModelKey::Paper(LayoutId(1)))
        );
        assert_eq!(
            ModelKey::from_persistent(PersistentId::expect(IdKind::BlockModel, 1)),
            Some(ModelKey::Block(BlockId(1)))
        );
    }
}
