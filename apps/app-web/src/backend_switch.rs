//! Browser backend-switch resolution, free of UI/browser types so host tests
//! exercise the exact decision logic.
//!
//! `SwitchBackend` is a browser-only command: the shared engine refuses it, so
//! the web composition root intercepts it in its `EngineClient` wrapper. The
//! decision logic — resolve the payload, persist the preference, then reload —
//! lives here, outside the wasm-gated `browser` module, with the storage and
//! reload side effects injected as closures. Every fallible step is distinct and
//! propagated: a foreign/absent payload never stores or reloads, a store failure
//! never reloads, and a reload failure is surfaced. There is no fake success.

use cad_app::BackendChoice;
use cad_domain::CadResult;
use cad_engine::render::BackendPreference;

/// Map a UI backend choice to the session's backend preference.
pub(crate) fn preference_for(choice: BackendChoice) -> BackendPreference {
    match choice {
        BackendChoice::Auto => BackendPreference::Auto,
        BackendChoice::WebGpu => BackendPreference::WebGpu,
        BackendChoice::WebGl2 => BackendPreference::WebGl2,
    }
}

/// Resolve the preference carried by a `SwitchBackend` command payload.
///
/// Retained (test-only) as the P3 payload-shaped contract; the P4 traced-bridge
/// path reads the `Copy` choice up front and uses [`switch_backend_for_choice`],
/// so this production-unused helper is compiled only for tests.
///
/// A foreign or absent payload is an explicit error, never a silent success.
#[cfg(test)]
pub(crate) fn preference_from_payload(
    payload: &cad_app::CommandPayload,
) -> CadResult<BackendPreference> {
    match payload {
        cad_app::CommandPayload::Backend(choice) => Ok(preference_for(*choice)),
        _ => Err(cad_domain::CadError::InvalidInput(
            "SwitchBackend requires a backend payload".into(),
        )),
    }
}

/// Persist the chosen preference from a payload, then reload.
///
/// Retained (test-only) as the P3 payload-shaped contract; the P4 traced-bridge
/// path uses [`switch_backend_for_choice`] because the command payload is moved
/// into the host bridge. `store`/`reload` are `FnOnce` so a step that is never
/// reached cannot be observed as success.
#[cfg(test)]
pub(crate) fn switch_backend(
    payload: &cad_app::CommandPayload,
    store: impl FnOnce(BackendPreference) -> CadResult<()>,
    reload: impl FnOnce() -> CadResult<()>,
) -> CadResult<()> {
    let preference = preference_from_payload(payload)?;
    store(preference)?;
    reload()
}

/// Persist the chosen preference (already read as a `Copy` choice), then reload.
///
/// The traced-bridge path reads the `BackendChoice` out of the command before the
/// command moves into the host, so the action closure only owns the `Copy` choice.
/// Same distinct/propagated failure semantics as [`switch_backend`].
pub(crate) fn switch_backend_for_choice(
    choice: BackendChoice,
    store: impl FnOnce(BackendPreference) -> CadResult<()>,
    reload: impl FnOnce() -> CadResult<()>,
) -> CadResult<()> {
    store(preference_for(choice))?;
    reload()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_app::CommandPayload;
    use cad_domain::CadError;
    use std::cell::Cell;

    #[test]
    fn foreign_payload_is_rejected_without_storing_or_reloading() {
        let stored = Cell::new(false);
        let reloaded = Cell::new(false);
        let error = switch_backend(
            &CommandPayload::None,
            |_| {
                stored.set(true);
                Ok(())
            },
            || {
                reloaded.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, CadError::InvalidInput(_)));
        assert!(!stored.get(), "a foreign payload must never store");
        assert!(!reloaded.get(), "a foreign payload must never reload");
    }

    #[test]
    fn store_failure_is_propagated_and_does_not_reload() {
        let reloaded = Cell::new(false);
        let error = switch_backend(
            &CommandPayload::Backend(BackendChoice::WebGl2),
            |_| Err(CadError::Unsupported("localStorage unavailable".into())),
            || {
                reloaded.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, CadError::Unsupported(_)));
        assert!(!reloaded.get(), "a store failure must never reload");
    }

    #[test]
    fn reload_failure_is_propagated_after_the_store_succeeded() {
        let stored = Cell::new(None);
        let error = switch_backend(
            &CommandPayload::Backend(BackendChoice::WebGpu),
            |preference| {
                stored.set(Some(preference));
                Ok(())
            },
            || Err(CadError::Invariant("reload failed".into())),
        )
        .unwrap_err();
        assert!(matches!(error, CadError::Invariant(_)));
        assert_eq!(stored.get(), Some(BackendPreference::WebGpu));
    }

    #[test]
    fn choice_switch_stores_the_resolved_preference_then_reloads() {
        let stored = Cell::new(None);
        let reloaded = Cell::new(false);
        switch_backend_for_choice(
            BackendChoice::WebGpu,
            |preference| {
                stored.set(Some(preference));
                Ok(())
            },
            || {
                reloaded.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(stored.get(), Some(BackendPreference::WebGpu));
        assert!(reloaded.get());
    }

    #[test]
    fn choice_switch_propagates_store_failure_without_reloading() {
        let reloaded = Cell::new(false);
        let error = switch_backend_for_choice(
            BackendChoice::Auto,
            |_| Err(CadError::Unsupported("localStorage unavailable".into())),
            || {
                reloaded.set(true);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, CadError::Unsupported(_)));
        assert!(!reloaded.get(), "a store failure must never reload");
    }

    #[test]
    fn success_stores_the_resolved_preference_then_reloads() {
        let stored = Cell::new(None);
        let reloaded = Cell::new(false);
        switch_backend(
            &CommandPayload::Backend(BackendChoice::Auto),
            |preference| {
                stored.set(Some(preference));
                Ok(())
            },
            || {
                reloaded.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(stored.get(), Some(BackendPreference::Auto));
        assert!(reloaded.get(), "a successful switch reloads");
    }

    #[test]
    fn preference_from_payload_maps_every_choice_and_rejects_the_rest() {
        assert_eq!(
            preference_from_payload(&CommandPayload::Backend(BackendChoice::Auto)).unwrap(),
            BackendPreference::Auto
        );
        assert_eq!(
            preference_from_payload(&CommandPayload::Backend(BackendChoice::WebGpu)).unwrap(),
            BackendPreference::WebGpu
        );
        assert_eq!(
            preference_from_payload(&CommandPayload::Backend(BackendChoice::WebGl2)).unwrap(),
            BackendPreference::WebGl2
        );
        assert!(preference_from_payload(&CommandPayload::None).is_err());
    }
}
