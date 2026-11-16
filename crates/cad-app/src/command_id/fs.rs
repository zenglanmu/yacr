//! `FS` (AutoCAD Express Tools): select every object that touches the selected
//! object, optionally following a **tangent chain**.
//!
//! `FS` is command-line only (Express has no ribbon button for it) and takes no
//! payload: the current selection is the whole input and must hold **exactly one**
//! directly addressable model-space seed. The pass mutates the session selection
//! only — no transaction, no revision bump and no undo step.
//!
//! # `FSMODE` (the two modes)
//!
//! [`crate::SessionState`]'s session `FSMODE` token
//! (`cad-app`'s `drafting_extras::DraftingExtras::fs_mode`) selects the pass:
//!
//! * `FSMODE=OFF` (the default, AutoCAD's initial value) adds only entities that
//!   touch the seed **directly** — a crossing and a tangent contact both count,
//!   and nothing is transitive;
//! * `FSMODE=ON` follows the transitive **tangent** chain: each step must be a
//!   tangent contact (parallel or antiparallel tangents at the contact point, within
//!   `1e-3` rad), so a chain of fillet-like contacts is selected as one set.
//!
//! The implementation of both passes is
//! [`crate::app_drawing::fs_chain`] (module-private); this module is the contract.
//!
//! # Declared limits
//!
//! * only curve kinds participate (`LINE`, `CIRCLE`, `ARC`, `ELLIPSE`,
//!   `LWPOLYLINE`, `SPLINE`); text, hatch, dimensions, images, meshes and opaque
//!   data are not touch candidates and are skipped;
//! * only directly addressable **model-space** entities are enumerated — block
//!   instance members are not expanded and hidden layers/entities are skipped;
//! * `FSMODE=ON` classifies only links to entities that are not already in the
//!   chain, so an ordinary crossing between two chained entities does not abort the
//!   pass;
//! * the tangent is evaluated analytically per kind, never from a display
//!   tessellation; parallel **and** antiparallel tangents count as one tangent line
//!   (an orientation-reversing cusp is not distinguished from a smooth
//!   continuation);
//! * the typed `FSMODE ON|OFF` command line is **not wired** in this build: the
//!   argument grammar lives in `cad-ui-core`'s session layer, and no selector maps
//!   to the setter yet. The stored token is what `FS` reads.
//!
//! # Refusals (all named, all before the selection changes)
//!
//! * `fs.selection` — the selection is not exactly one object;
//! * `fs.not_addressable` — the seed is a block-instance member or sub-element;
//! * `fs.seed_missing` — the seed is not present in the drawing;
//! * `fs.seed_not_model_space` — the seed is not model space;
//! * `fs.seed_geometry` — the seed kind is not a curve `FS` intersects;
//! * `fs.intersect` — an engine intersection failed for a candidate;
//! * `fs.chain_nontangent` (`FSMODE=ON`) — a contact is not tangential, or its
//!   tangent cannot be resolved;
//! * `fs.chain_ambiguous` (`FSMODE=ON`) — two distinct candidates are tangent to the
//!   same entity at the same contact point, so the continuation is undefined;
//! * `fs.chain_bound` (`FSMODE=ON`) — the chain exceeds the documented bound of
//!   `FS_CHAIN_MAX_ENTITIES` (64) entities; it is refused, never truncated;
//! * `fs.chain_not_addressable` (`FSMODE=ON`) — an entity of the chain is no
//!   longer present between steps (defensive: a static drawing cannot trigger it).
//!
//! A command that finds no contact is **not** a failure: it reports `fs.none` and
//! leaves the selection unchanged. A successful pass reports `fs.applied` and, with
//! it, `fs.mode` naming the pass, its entity count and its contact count.
