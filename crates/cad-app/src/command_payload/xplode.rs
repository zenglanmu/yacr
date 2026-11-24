//! `XPLODE`'s typed controls, split out of `payload_extra.rs` for the code-size
//! gate.

use super::payload_extra::EntityPropertyWrite;

/// `XPLODE`'s typed controls.
///
/// AutoCAD's XPLODE options are `All` / `Color` / `LAyer` / `LType` /
/// `LWeight` / `Inherit from parent block` / `Explode`. This build reaches the
/// same state through two fields instead of a prompt sequence:
///
/// * `properties` is the ordered `layer`/`color`/`linetype`/`lineweight` list
///   (`All` is the list carrying all four; `Explode` is the empty list). Any
///   other key is refused **by name** — XPLODE does not offer `transparency`,
///   and `ltscale`/`thickness`/`material`/plot style are not modelled.
/// * `inherit_from_parent` is the `Inherit from parent block` option: children
///   on layer `0` move to the reference's layer and `ByBlock` colour / linetype
///   / lineweight resolve against the reference.
///
/// The two are mutually exclusive, exactly like the AutoCAD prompt: choosing a
/// property replaces the inherit option. `Individually` has no payload form
/// (this build has no interactive prompt; the command applies one control set
/// globally). Attribute values are not a control: XPLODE drops them and the
/// block definition's `ATTDEF` members are emitted, the same as `EXPLODE`
/// (`BURST` is the value-preserving command).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XPlodeSettings {
    pub inherit_from_parent: bool,
    pub properties: Vec<EntityPropertyWrite>,
}
