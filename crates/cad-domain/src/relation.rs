//! Typed relation policy and in-context state (ADR-0043 D1/D2, P2/P3).
//!
//! This module owns the **vocabulary** of relation families, the reference-rule
//! registry and the in-context state machine. It is deliberately pure: the
//! persistent relation tables and their codecs live in `cad-db`, exactly like
//! `BimRelations`.
//!
//! * [`RelationFamily`] is the D1 split: ownership edges never propagate, value
//!   edges always do (through the parameter graph, not here), and reference
//!   edges propagate **only** when a `(host_type, hosted_type)` rule is
//!   registered. P2 registers no rule, so `voids`/`fills` are
//!   [`Propagation::NoRule`] and report `rel.no_rule` instead of inventing a
//!   wall-reflow heuristic.
//! * [`InContextState`] is the P3 state machine for a self-authored in-context
//!   face/edge/vertex reference. Transitions are explicit; a state that cannot
//!   move to the requested one is `rel.state_transition`.

/// A reference edge whose host/hosted pair has no registered rule.
pub const REL_NO_RULE: &str = "rel.no_rule";
/// An in-context relation transition the state machine does not allow.
pub const REL_STATE_TRANSITION: &str = "rel.state_transition";
/// A feature/relation operation this build cannot evaluate (P2 boundary).
pub const REL_NOT_IMPLEMENTED: &str = "rel.not_implemented";

/// The three D1 relation families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationFamily {
    /// Identity/containment (`contains`, `nests`, `aggregates`, `group`,
    /// definition → instance). Never part of a re-evaluation closure.
    Ownership,
    /// Value/definition (parameter references, slot bindings, feature inputs).
    /// Always part of the closure.
    Value,
    /// Host/reference (`voids`, `fills`, in-context face/edge/vertex). Part of
    /// the closure only when a `(host, hosted)` rule is registered.
    Reference,
}

/// What one relation/propagation kind does during a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Propagation {
    /// Ownership: no propagation.
    Never,
    /// Reference with a registered rule: propagate.
    Rule,
    /// A feature handler will evaluate it (P2 `feature` pipeline).
    Handler,
    /// A reference kind without a rule: stable `rel.no_rule`, no propagation.
    NoRule,
}

/// The D1 family of a persisted relation token.
///
/// `other:<token>` is always [`RelationFamily::Ownership`]: the IFC exporter
/// already reports it as not-written, so it cannot carry propagation semantics.
pub fn relation_family(kind_token: &str) -> RelationFamily {
    match kind_token {
        "contains" | "nests" | "aggregates" | "group" | "connects" => RelationFamily::Ownership,
        "voids" | "fills" => RelationFamily::Reference,
        _ if kind_token.starts_with("other:") => RelationFamily::Ownership,
        // An unknown token is treated as ownership (no propagation) rather than
        // silently inventing a reference rule.
        _ => RelationFamily::Ownership,
    }
}

/// The propagation policy of a persisted relation kind (D2).
///
/// P2 registers **no** `(host, hosted)` rule: `voids`/`fills` therefore return
/// [`Propagation::NoRule`] regardless of the type keys. The signature carries
/// the type keys so a future registry can be the only change.
pub fn relation_propagation(
    kind_token: &str,
    _host_type_key: &str,
    _hosted_type_key: &str,
) -> Propagation {
    match relation_family(kind_token) {
        RelationFamily::Ownership => Propagation::Never,
        RelationFamily::Value => Propagation::Rule,
        RelationFamily::Reference => Propagation::NoRule,
    }
}

/// A self-authored in-context reference path (P3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationReference {
    Face { key: String },
    Edge { key: String },
    Vertex { key: String },
}

impl RelationReference {
    pub fn token(&self) -> &'static str {
        match self {
            Self::Face { .. } => "face",
            Self::Edge { .. } => "edge",
            Self::Vertex { .. } => "vertex",
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Face { key } | Self::Edge { key } | Self::Vertex { key } => key,
        }
    }

    pub fn parse(token: &str, key: String) -> Option<Self> {
        match token {
            "face" => Some(Self::Face { key }),
            "edge" => Some(Self::Edge { key }),
            "vertex" => Some(Self::Vertex { key }),
            _ => None,
        }
    }
}

/// The P3 in-context state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InContextState {
    /// The reference resolves in the current document and participates.
    InContext,
    /// The reference still exists but is outside the active context.
    OutOfContext,
    /// The reference is intentionally frozen.
    Locked,
    /// The reference target failed (geometry/kernel) and keeps last-good.
    Broken,
    /// The target is gone; the edge is retained as an explicit dangling record.
    Dangling,
}

impl InContextState {
    pub fn token(self) -> &'static str {
        match self {
            Self::InContext => "in_context",
            Self::OutOfContext => "out_of_context",
            Self::Locked => "locked",
            Self::Broken => "broken",
            Self::Dangling => "dangling",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "in_context" => Some(Self::InContext),
            "out_of_context" => Some(Self::OutOfContext),
            "locked" => Some(Self::Locked),
            "broken" => Some(Self::Broken),
            "dangling" => Some(Self::Dangling),
            _ => None,
        }
    }

    /// Whether a state change is legal.
    ///
    /// `Dangling` is terminal: a missing target is not silently revived by a
    /// later state write; the edge must be deleted and recreated. `Locked` may
    /// return to `InContext`/`OutOfContext`; `Broken` may recover to either or
    /// become `Dangling`.
    pub fn can_transition(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::InContext => matches!(
                next,
                Self::OutOfContext | Self::Locked | Self::Broken | Self::Dangling
            ),
            Self::OutOfContext => matches!(
                next,
                Self::InContext | Self::Locked | Self::Broken | Self::Dangling
            ),
            Self::Locked => matches!(next, Self::InContext | Self::OutOfContext),
            Self::Broken => matches!(next, Self::InContext | Self::OutOfContext | Self::Dangling),
            Self::Dangling => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_follow_d1() {
        assert_eq!(relation_family("contains"), RelationFamily::Ownership);
        assert_eq!(relation_family("voids"), RelationFamily::Reference);
        assert_eq!(relation_family("fills"), RelationFamily::Reference);
        assert_eq!(relation_family("other:custom"), RelationFamily::Ownership);
        assert_eq!(relation_family("unknown"), RelationFamily::Ownership);
    }

    #[test]
    fn reference_kinds_without_a_rule_do_not_propagate() {
        assert_eq!(
            relation_propagation("voids", "IfcWall", "IfcDoor"),
            Propagation::NoRule
        );
        assert_eq!(
            relation_propagation("contains", "IfcBuildingStorey", "IfcWall"),
            Propagation::Never
        );
    }

    #[test]
    fn dangling_is_terminal_and_other_states_can_recover() {
        assert!(!InContextState::Dangling.can_transition(InContextState::InContext));
        assert!(InContextState::Broken.can_transition(InContextState::InContext));
        assert!(InContextState::Locked.can_transition(InContextState::OutOfContext));
        assert!(!InContextState::Locked.can_transition(InContextState::Broken));
        assert!(InContextState::InContext.can_transition(InContextState::InContext));
    }

    #[test]
    fn reference_tokens_round_trip() {
        for reference in [
            RelationReference::Face { key: "f1".into() },
            RelationReference::Edge { key: "e1".into() },
            RelationReference::Vertex { key: "v1".into() },
        ] {
            let decoded = RelationReference::parse(reference.token(), reference.key().into());
            assert_eq!(decoded, Some(reference));
        }
        assert_eq!(RelationReference::parse("solid", "x".into()), None);
    }
}
