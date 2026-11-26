//! P-MESH-1b selector contracts: `MESHREPAIR` / `MESHREBUILD`.

use super::super::*;
use cad_app::ExtendedCommandId;

#[test]
fn mesh_repair_selectors_are_admitted_once_and_work_only() {
    for (selector, id) in [
        ("MESHREPAIR", ExtendedCommandId::MeshRepair),
        ("MESHREBUILD", ExtendedCommandId::MeshRebuild),
        ("MESHREFINE", ExtendedCommandId::MeshRefine),
        ("MESHSMOOTH", ExtendedCommandId::MeshSmooth),
        ("MESHCAP", ExtendedCommandId::MeshCap),
    ] {
        assert_eq!(
            normalize_command(selector),
            Some(CommandId::Extended(id)),
            "{selector}"
        );
        assert_eq!(
            canonical_command_name(selector),
            Some(selector),
            "{selector}"
        );
        assert_eq!(
            COMMAND_TABLE
                .iter()
                .filter(|(name, _)| *name == selector)
                .count(),
            1,
            "{selector} must appear exactly once in COMMAND_TABLE"
        );
        assert!(id.requires_work_mode(), "{selector} writes the drawing");
    }
}
