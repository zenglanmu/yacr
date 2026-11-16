//! The shared create funnel's input types and session-colour resolution.
//!
//! Split out of `trim_extend.rs` (2026-11-16) so both files stay inside the
//! 1000-line code gate; nothing here changed behaviour. `NewEntityInsert` is the
//! bundle the funnel takes instead of six positional parameters, and
//! `new_entity_color` is the one place the session `COLOR` token becomes a
//! stored display colour.

use super::*;

/// The per-insertion options of the shared create funnel
/// ([`Application::insert_new_entities`]).
///
/// Bundled instead of six positional parameters so the stored `type_key` override
/// fits without a second, near-identical funnel. `type_key: None` is exactly the
/// pre-existing behaviour ([`class_name`]'s answer).
pub(super) struct NewEntityInsert<'a> {
    pub(super) layer: LayerId,
    pub(super) space: SpaceId,
    /// A captured display state (render attributes + dynamic-visibility
    /// membership); only the single-entity OFFSET path captures one.
    pub(super) display: Option<cad_db::EntityDisplayState>,
    /// The session `COLOR` colour stamped on the inserted rows; `None` stages no
    /// display state at all, which is exactly the pre-`COLOR` behaviour (the
    /// entity keeps the symbolic `ByLayer` default).
    pub(super) color: Option<cad_db::EntityColor>,
    /// The stored `type_key` override; `None` = [`class_name`].
    pub(super) type_key: Option<&'a str>,
    /// The transaction / undo label (also used in refusal messages).
    pub(super) label: &'a str,
}

/// The session `COLOR` colour the create funnel stamps on a new entity.
///
/// `256` is AutoCAD's ByLayer token and `0` its ByBlock token. ByLayer is the
/// **absence** of a per-entity override, so it returns `None` and the inserted
/// row keeps exactly the pre-`COLOR` behaviour; `0` stays symbolic (`ByBlock`)
/// because the block-reference resolution is the model's, not this funnel's.
/// `1..=255` resolves through the one ACI table the build owns
/// ([`crate::property_value::rgb_for_aci`]) — the same table
/// `SETENTITYPROPERTY color aci:N` writes — so a created entity and a recoloured
/// one store the identical value and both export the same way.
///
/// The token's closed domain is enforced by
/// [`crate::session_state::DraftingExtras::set_current_color`], so an
/// unresolvable index here is an invariant violation, never a silent fallback to
/// ByLayer.
pub(super) fn new_entity_color(session: &SessionState) -> CadResult<Option<cad_db::EntityColor>> {
    match session.drafting_extras.current_color() {
        256 => Ok(None),
        0 => Ok(Some(cad_db::EntityColor::ByBlock)),
        aci => crate::property_value::rgb_for_aci(aci as u16)
            .map(|rgb| Some(cad_db::EntityColor::Explicit(rgb)))
            .ok_or_else(|| {
                CadError::Invariant(format!(
                    "color.invalid_value: the session COLOR token {aci} is outside the ACI palette \
                     this build resolves, so a created entity's colour cannot be derived from it"
                ))
            }),
    }
}
