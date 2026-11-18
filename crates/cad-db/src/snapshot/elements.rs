//! Elements row codecs: `serde_json::Value` ⇄ rows.
//!
//! Crate-private on purpose: only [`super::DocumentSnapshot`] crosses the store
//! boundary, so a store can never invent a payload shape the database does not
//! understand.
