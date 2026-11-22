//! The compiled connector table this carrier links.
//!
//! The shipped **browser** bundle (`scripts/build-web.sh`) is built from
//! `app-web` alone, and every `cad-app` edge in its graph sets
//! `default-features = false` (see this crate and
//! `cad-engine`/`cad-ui-core`/`cad-host` manifests), so it carries only the
//! DWG/DXF lanes. The desktop `dom-canvas` build (`scripts/build-canvas-wasm.sh`)
//! passes `--features full-formats` and keeps every lane.
//!
//! **`cargo test --workspace` unifies features with the native carriers**, so in
//! that run this test sees `formats-all` and only checks that the DWG/DXF lanes
//! are present. The shipped-artifact guard is the build itself:
//! `scripts/build-web.sh` builds `-p app-web` alone, and
//! `scripts/lib/wasm-toolchain.sh` fails closed on a `cad-app` `formats-*` leak or
//! a linked connector crate. `cargo test -p app-web` is the run that exercises the
//! slim table.

use cad_app::{ConnectorRegistry, ALL_FORMATS_COMPILED};

#[test]
fn the_browser_registry_keeps_the_dwg_and_dxf_lanes() {
    let ids: Vec<&str> = ConnectorRegistry::builtin()
        .descriptors()
        .map(|descriptor| descriptor.id)
        .collect();
    assert!(ids.contains(&"dwg"), "no DWG lane: {ids:?}");
    assert!(ids.contains(&"dxf"), "no DXF lane: {ids:?}");
    if !ALL_FORMATS_COMPILED {
        assert_eq!(
            ids,
            ["dwg", "dxf"],
            "the slim table must be exactly DWG/DXF"
        );
    }
}
