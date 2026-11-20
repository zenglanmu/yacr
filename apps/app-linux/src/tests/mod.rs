//! Carrier-level tests that need something a shared `cad-*` layer cannot have.
//!
//! `ymodel_texture` is the end-to-end half of ADR-0038 D3/D4: it writes a real
//! `.ymodel` **file** through `cad-store-zip` and then looks at that file as a
//! zip, which is the only way to assert the `textures/<key>` entry name. The
//! shared layers see snapshots; only this carrier links the container crate.

#[cfg(target_os = "linux")]
mod ymodel_texture;
