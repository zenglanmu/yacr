//! Pure host policy: command routing, status ownership and export-format
//! classification. Every function here is free of UI/GPU/platform state, so
//! it is unit-testable in isolation.
use cad_app::CommandId;
use cad_platform::FileTarget;

/// The stable refusal a carrier without a native document archive reports.
///
/// Not a localized sentence: the marker is the contract (ADR-0037 D13, the same
/// string the carrier adapters emit), so an open or a save that cannot run never
/// silently falls back to a connector format.
pub const UNSUPPORTED_YMODEL: &str = "unsupported-runtime: ymodel";

/// Plot, Save As and Export own their own success/failure status
/// (`plot.exported` / `save.exported` / ... / `export.picker_waiting`); the
/// generic completed/failed text must not overwrite it.
pub fn owns_status(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::PlotDrawing | CommandId::SaveDrawingAs | CommandId::Export
    )
}

/// The lowercased extension of a target name, or `""` when there is none.
pub fn extension_of(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// A drawing-name hint reduced to a non-empty file stem, else `"drawing"`.
pub fn default_stem(hint: &str) -> String {
    std::path::Path::new(hint)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("drawing")
        .to_string()
}

/// The suggested save file name for a drawing hint and extension.
pub fn default_save_name(hint: &str, extension: &str) -> String {
    format!("{}.{}", default_stem(hint), extension)
}

/// Whether a target denotes the native `.ymodel` document archive.
/// Decided from the user-facing name and the carrier handle, case-insensitively:
/// the desktop handle is a path, Android's is a SAF `content://` URI and the
/// browser's is a download label, so the name is the only field all three share.
/// This decides which *pipeline* runs (archive decode/encode vs. connector
/// import/export) — never which bytes a file holds.
pub fn is_ymodel_target(target: &FileTarget) -> bool {
    is_ymodel_name(&target.display_name) || is_ymodel_name(&target.opaque)
}

/// Whether a name ends with `.ymodel` (case-insensitive).
pub fn is_ymodel_name(name: &str) -> bool {
    const SUFFIX: &[u8] = b".ymodel";
    let name = name.trim();
    // Byte comparison, not string slicing: a non-ASCII name must not panic on a
    // char boundary (it simply is not a `.ymodel`).
    name.len() >= SUFFIX.len()
        && name.as_bytes()[name.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
}

/// Classification of a lossy-save target by extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFormat {
    Dxf,
    /// Experimental binary DWG (upstream acadrust `DwgWriter`), always a new file.
    Dwg,
    /// A registry export key (`.stl`/`.obj`/`.ply`/`.off`/`.coff`/`.ifc`/`.skp`/
    /// `.glb`/`.gltf`).
    ///
    /// `.stl` is the **binary** STL container; the ASCII variant (`stl-ascii`)
    /// has no extension of its own and is reachable only through the registry
    /// API, never by typing a file name. `.coff` is the same ASCII OFF writer as
    /// `.off` (the COFF header variant), and `.glb`/`.gltf` are the two
    /// registry keys of the one `gltf` connector (binary and JSON containers).
    Connector(&'static str),
    NoExtension,
    Unknown(String),
}

/// Classify a lossy-save target's extension (already lowercased).
pub fn save_format(extension: &str) -> SaveFormat {
    match extension {
        "dxf" => SaveFormat::Dxf,
        "dwg" => SaveFormat::Dwg,
        "stl" => SaveFormat::Connector("stl"),
        "obj" => SaveFormat::Connector("obj"),
        "ply" => SaveFormat::Connector("ply"),
        "off" => SaveFormat::Connector("off"),
        // `.coff` is the colour-capable OFF spelling; the registry key is the
        // same ASCII OFF/COFF writer (`off`), not a second descriptor.
        "coff" => SaveFormat::Connector("off"),
        "ifc" => SaveFormat::Connector("ifc"),
        "skp" => SaveFormat::Connector("skp"),
        // glTF: `.glb` is the binary container key and `.gltf` the JSON one; both
        // belong to the one `gltf` descriptor.
        "glb" => SaveFormat::Connector("glb"),
        "gltf" => SaveFormat::Connector("gltf"),
        "" => SaveFormat::NoExtension,
        other => SaveFormat::Unknown(other.to_string()),
    }
}

/// One `EXPORT <TOKEN>` spelling and the writer it names.
///
/// `token` is the typed word (matched ASCII case-insensitively), `format_id` is
/// the `cad-app` connector-registry key the host dispatches, and `extension` is
/// the extension the picker suggests for it. The two are not always the same
/// spelling: `COFF` is the colour-capable OFF header written by the one `off`
/// registry descriptor under a `.coff` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFormatSpec {
    pub token: &'static str,
    pub format_id: &'static str,
    pub extension: &'static str,
}

/// Every format a typed `EXPORT <TOKEN>` can select, in the documented order.
///
/// The bare `EXPORT` picker is still the all-format chooser: this table only
/// names the ones a typed token may force. A token outside it is refused with a
/// stable reason (never silently mapped onto another writer).
pub const EXPORT_FORMATS: &[ExportFormatSpec] = &[
    ExportFormatSpec {
        token: "DXF",
        format_id: "dxf",
        extension: "dxf",
    },
    ExportFormatSpec {
        token: "DWG",
        format_id: "dwg",
        extension: "dwg",
    },
    ExportFormatSpec {
        token: "STL",
        format_id: "stl",
        extension: "stl",
    },
    ExportFormatSpec {
        token: "OBJ",
        format_id: "obj",
        extension: "obj",
    },
    ExportFormatSpec {
        token: "PLY",
        format_id: "ply",
        extension: "ply",
    },
    ExportFormatSpec {
        token: "OFF",
        format_id: "off",
        extension: "off",
    },
    ExportFormatSpec {
        token: "COFF",
        format_id: "off",
        extension: "coff",
    },
    ExportFormatSpec {
        token: "IFC",
        format_id: "ifc",
        extension: "ifc",
    },
    ExportFormatSpec {
        token: "SKP",
        format_id: "skp",
        extension: "skp",
    },
    ExportFormatSpec {
        token: "GLB",
        format_id: "glb",
        extension: "glb",
    },
    ExportFormatSpec {
        token: "GLTF",
        format_id: "gltf",
        extension: "gltf",
    },
];

/// Resolve one typed `EXPORT <TOKEN>` word, trimmed and ASCII case-insensitive.
pub fn export_format_for_token(token: &str) -> Option<&'static ExportFormatSpec> {
    let token = token.trim();
    EXPORT_FORMATS
        .iter()
        .find(|spec| spec.token.eq_ignore_ascii_case(token))
}

/// The format a document opened from `name` would be written back as, when that
/// is a **non-experimental write-back** format the bare `EXPORT` picker may
/// suggest as its own extension; `None` means the caller falls back to DXF.
///
/// `.dwg` and `.skp` are excluded on purpose: both writers are experimental
/// (ADR-0041 D5 for DWG, the SKP descriptor for SketchUp) and a picker default
/// must not steer an ordinary export into an experimental writer. `.ymodel`,
/// a blank/unknown name and the read-only formats (STEP/DGN/RVT) have no
/// writable entry in [`EXPORT_FORMATS`], so they fall back as well.
pub fn export_format_for_source(name: &str) -> Option<&'static ExportFormatSpec> {
    let extension = extension_of(name);
    match save_format(&extension) {
        // Experimental writers are never a suggested default (ADR-0041 D5).
        SaveFormat::Dwg => None,
        SaveFormat::Connector("skp") => None,
        // DXF and every other writable connector extension map to the spec whose
        // own extension is exactly this one (`.coff` keeps its own spelling).
        SaveFormat::Dxf | SaveFormat::Connector(_) => EXPORT_FORMATS
            .iter()
            .find(|spec| spec.extension == extension.as_str()),
        // No extension, `.ymodel`, and read-only/unknown sources.
        SaveFormat::NoExtension | SaveFormat::Unknown(_) => None,
    }
}

/// Classification of a vector-plot target by extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlotFormat {
    Svg,
    Pdf,
    /// `.png` needs surface readback, which only the CLI performs: refused.
    Png,
    NoExtension,
    Unknown(String),
}

/// Classify a vector-plot target's extension (already lowercased).
pub fn plot_format(extension: &str) -> PlotFormat {
    match extension {
        "svg" => PlotFormat::Svg,
        "pdf" => PlotFormat::Pdf,
        "png" => PlotFormat::Png,
        "" => PlotFormat::NoExtension,
        other => PlotFormat::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plot_save_and_export_own_their_status() {
        assert!(owns_status(CommandId::PlotDrawing));
        assert!(owns_status(CommandId::SaveDrawingAs));
        assert!(owns_status(CommandId::Export));
        assert!(!owns_status(CommandId::OpenDrawing));
        assert!(!owns_status(CommandId::NewDrawing));
        assert!(!owns_status(CommandId::FitDrawing));
        assert!(!owns_status(CommandId::CancelLoading));
    }

    #[test]
    fn extension_is_lowercased_and_path_aware() {
        assert_eq!(extension_of("/tmp/Out.SVG"), "svg");
        assert_eq!(extension_of("drawing"), "");
        assert_eq!(extension_of("a.b.c"), "c");
    }

    #[test]
    fn default_names_use_the_stem_or_drawing() {
        assert_eq!(default_stem("flange.dxf"), "flange");
        assert_eq!(default_stem("/tmp/plan.dwg"), "plan");
        assert_eq!(default_stem(""), "drawing");
        assert_eq!(default_stem("yacr-demo"), "yacr-demo");
        assert_eq!(default_save_name("flange.dxf", "svg"), "flange.svg");
        assert_eq!(default_save_name("", "dxf"), "drawing.dxf");
        // SAVE always targets the native archive, whatever the source was.
        assert_eq!(default_save_name("/tmp/plan.dwg", "ymodel"), "plan.ymodel");
        assert_eq!(default_save_name("", "ymodel"), "drawing.ymodel");
    }

    #[test]
    fn ymodel_targets_are_recognized_case_insensitively_from_either_field() {
        assert!(is_ymodel_name("plan.ymodel"));
        assert!(is_ymodel_name("/tmp/PLAN.YMODEL"));
        assert!(
            is_ymodel_name("plan.ymodel "),
            "a trailing space is trimmed"
        );
        assert!(!is_ymodel_name("plan.ymodel.bak"));
        assert!(!is_ymodel_name("plan.dwg"));
        assert!(!is_ymodel_name("plan"));
        assert!(!is_ymodel_name("ymodel"), "the dot is required");
        // A non-ASCII name must be a plain `false`, never a slicing panic.
        assert!(!is_ymodel_name("图纸"));
        assert!(!is_ymodel_name("图纸文件"));
        // The carrier handle counts too: a SAF URI whose display name was lost.
        assert!(is_ymodel_target(&FileTarget {
            display_name: String::new(),
            opaque: "content://provider/document/plan.YMODEL".to_string(),
        }));
        assert!(!is_ymodel_target(&FileTarget::from_path("/tmp/plan.dxf")));
    }

    #[test]
    fn save_and_plot_formats_grade_every_branch() {
        assert_eq!(save_format("dxf"), SaveFormat::Dxf);
        assert_eq!(save_format("dwg"), SaveFormat::Dwg);
        // Every connector writer key is reachable by its extension; `.stl` is
        // the binary container (ASCII has no extension of its own).
        assert_eq!(save_format("stl"), SaveFormat::Connector("stl"));
        assert_eq!(save_format("obj"), SaveFormat::Connector("obj"));
        assert_eq!(save_format("ply"), SaveFormat::Connector("ply"));
        assert_eq!(save_format("off"), SaveFormat::Connector("off"));
        // `.coff` is an extension of the same ASCII writer, not a new key.
        assert_eq!(save_format("coff"), SaveFormat::Connector("off"));
        assert_eq!(save_format("ifc"), SaveFormat::Connector("ifc"));
        assert_eq!(save_format("skp"), SaveFormat::Connector("skp"));
        // Both glTF containers are registry keys of the one `gltf` descriptor.
        assert_eq!(save_format("glb"), SaveFormat::Connector("glb"));
        assert_eq!(save_format("gltf"), SaveFormat::Connector("gltf"));
        assert_eq!(save_format(""), SaveFormat::NoExtension);
        assert_eq!(save_format("txt"), SaveFormat::Unknown("txt".into()));

        assert_eq!(plot_format("svg"), PlotFormat::Svg);
        assert_eq!(plot_format("pdf"), PlotFormat::Pdf);
        assert_eq!(plot_format("png"), PlotFormat::Png);
        assert_eq!(plot_format(""), PlotFormat::NoExtension);
        assert_eq!(plot_format("txt"), PlotFormat::Unknown("txt".into()));
    }

    #[test]
    fn export_tokens_resolve_case_insensitively_to_their_writer() {
        for (token, format_id, extension) in [
            ("DXF", "dxf", "dxf"),
            ("DWG", "dwg", "dwg"),
            ("STL", "stl", "stl"),
            ("OBJ", "obj", "obj"),
            ("PLY", "ply", "ply"),
            ("OFF", "off", "off"),
            ("COFF", "off", "coff"),
            ("IFC", "ifc", "ifc"),
            ("SKP", "skp", "skp"),
            ("GLB", "glb", "glb"),
            ("GLTF", "gltf", "gltf"),
        ] {
            let spec = export_format_for_token(token).expect("a documented token");
            assert_eq!((spec.format_id, spec.extension), (format_id, extension));
            // Surrounding whitespace and any ASCII case spelling are the same token.
            assert_eq!(
                export_format_for_token(&format!("  {}  ", token.to_ascii_lowercase()))
                    .map(|spec| spec.token),
                Some(token)
            );
        }
        assert!(export_format_for_token("ZIP").is_none());
        assert!(export_format_for_token("").is_none());
        assert!(export_format_for_token("gl tf").is_none());
    }

    #[test]
    fn a_source_extension_only_suggests_a_non_experimental_write_back_format() {
        // Writable, non-experimental: the source's own extension is suggested.
        for (name, extension) in [
            ("/tmp/plan.dxf", "dxf"),
            ("mesh.stl", "stl"),
            ("mesh.obj", "obj"),
            ("mesh.ply", "ply"),
            ("mesh.off", "off"),
            ("mesh.coff", "coff"),
            ("model.ifc", "ifc"),
            ("model.glb", "glb"),
            ("model.gltf", "gltf"),
        ] {
            assert_eq!(
                export_format_for_source(name).map(|spec| spec.extension),
                Some(extension),
                "{name}"
            );
        }
        // Experimental writers (ADR-0041 D5) and everything read-only/unknown
        // fall back, so the caller suggests `.dxf`.
        for name in [
            "/tmp/plan.dwg",
            "/tmp/plan.SKP",
            "plan.ymodel",
            "plan.step",
            "plan.dgn",
            "plan.rvt",
            "plan",
            "",
            "plan.txt",
        ] {
            assert!(
                export_format_for_source(name).is_none(),
                "{name} must fall back to dxf"
            );
        }
    }
}
