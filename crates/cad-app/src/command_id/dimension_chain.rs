//! Long-form contract for [`crate::CommandId::DimContinue`].
//!
//! `DIMCONTINUE` chains **N** linear dimensions from one picked **linear** base
//! dimension (ADR 0017 decision C, P5f).
//!
//! Payload is `DimChain { base, points }`. Segment 1 is `base.second → P_1`,
//! segment `i > 1` is `P_{i−1} → P_i`, and every segment keeps
//! `dimension_line = base.location` (a chain does **not** stack). The pick
//! order defines the chain direction: a segment whose projection along
//! `base`'s direction is `<= 0` is refused by name (no folding back). Every
//! other refusal matches [`crate::CommandId::DimBaseline`], and the N segments
//! are inserted in one transaction / one undo step. Non-associative. Work-only.
