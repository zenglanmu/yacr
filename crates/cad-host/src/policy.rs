//! Pure host policy: command routing, status ownership and export-format
//! classification. Every function here is free of UI/GPU/platform state, so
//! it is unit-testable in isolation.
use cad_app::CommandId;
use cad_platform::FileTarget;

/// Plot and Save As own their own success/failure status
/// (`plot.exported` / `save.exported` / ...); the generic completed/failed text
/// must not overwrite it.
pub fn owns_status(id: CommandId) -> bool {
    matches!(id, CommandId::PlotDrawing | CommandId::SaveDrawingAs)
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

/// The suggested name for a DWG **export**.
///
/// Deliberately different from the source name: a save from a `.dwg` document
/// writes a new file (`<stem>-export.dwg`) and never silently replaces the
/// opened drawing (DWG output is experimental and lossy).
pub fn default_dwg_export_name(hint: &str) -> String {
    format!("{}-export.dwg", default_stem(hint))
}

/// Whether the opened source name denotes a DWG drawing.
pub fn source_is_dwg(name: &str) -> bool {
    extension_of(name) == "dwg"
}

/// A lexical path/handle normalization used for the source-overwrite backstop.
///
/// Collapses `.` components and resolves `..` lexically (no filesystem access),
/// so `/tmp/./a.dwg` and `/tmp/a.dwg` compare equal. Case is preserved: on the
/// carriers where this backstop matters the filesystem is case-sensitive, and a
/// false refusal is safer than a false write (the platform's own
/// [`cad_platform::PlatformFiles::refuses_source_overwrite`] remains the
/// device+inode / URI identity guard).
fn normalized(name: &str) -> String {
    use std::path::{Component, Path};
    let mut out = std::path::PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// A non-empty handle that is a filesystem path (not a `scheme://` carrier URI).
fn is_path_handle(handle: &str) -> bool {
    !handle.is_empty() && !handle.contains("://")
}

/// Whether two save targets name the **same opened source** after normalization.
///
/// The host-side backstop for carriers where the platform cannot detect inode
/// identity (the web has neither path nor inode; Android has only a SAF URI):
/// two filesystem handles compare by normalized path, and any carrier handle
/// (a `download://` name, a `content://` URI, a label-only web open) compares by
/// normalized display name. It is intentionally conservative — refusing a
/// same-named target is safe; overwriting the source is not.
pub fn same_normalized_target(source: &FileTarget, target: &FileTarget) -> bool {
    if is_path_handle(&source.opaque) && is_path_handle(&target.opaque) {
        return normalized(&source.opaque) == normalized(&target.opaque);
    }
    let source_name = normalized(&source.display_name);
    let target_name = normalized(&target.display_name);
    !source_name.is_empty() && source_name == target_name
}

/// Classification of a lossy-save target by extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFormat {
    Dxf,
    /// Experimental binary DWG (upstream acadrust `DwgWriter`), always a new file.
    Dwg,
    NoExtension,
    Unknown(String),
}

/// Classify a lossy-save target's extension (already lowercased).
pub fn save_format(extension: &str) -> SaveFormat {
    match extension {
        "dxf" => SaveFormat::Dxf,
        "dwg" => SaveFormat::Dwg,
        "" => SaveFormat::NoExtension,
        other => SaveFormat::Unknown(other.to_string()),
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
    fn only_plot_and_save_own_their_status() {
        assert!(owns_status(CommandId::PlotDrawing));
        assert!(owns_status(CommandId::SaveDrawingAs));
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
        // A DWG save is always a NEW file with a distinct name.
        assert_eq!(default_dwg_export_name("/tmp/plan.dwg"), "plan-export.dwg");
        assert_eq!(default_dwg_export_name(""), "drawing-export.dwg");
        assert_ne!(default_dwg_export_name("plan.dwg"), "plan.dwg");
    }

    #[test]
    fn the_source_extension_selects_a_dwg_save() {
        assert!(source_is_dwg("plan.dwg"));
        assert!(source_is_dwg("/tmp/PLAN.DWG"));
        assert!(!source_is_dwg("plan.dxf"));
        assert!(!source_is_dwg("plan"));
    }

    #[test]
    fn normalized_source_identity_catches_path_and_name_variants() {
        let source = FileTarget::from_path("/tmp/./plan.dwg");
        assert!(same_normalized_target(
            &source,
            &FileTarget::from_path("/tmp/plan.dwg")
        ));
        assert!(!same_normalized_target(
            &source,
            &FileTarget::from_path("/tmp/other/plan.dwg")
        ));
        // A carrier handle (web download / Android SAF URI) compares names: the
        // platform cannot supply device+inode identity there.
        let web_source = FileTarget {
            display_name: "plan.dwg".into(),
            opaque: "plan.dwg".into(),
        };
        let web_target = FileTarget {
            display_name: "plan.dwg".into(),
            opaque: "download://plan.dwg".into(),
        };
        assert!(same_normalized_target(&web_source, &web_target));
        let android_source = FileTarget {
            display_name: "plan.dwg".into(),
            opaque: "content://tree/1".into(),
        };
        let android_target = FileTarget {
            display_name: "plan.dwg".into(),
            opaque: "content://tree/2".into(),
        };
        assert!(same_normalized_target(&android_source, &android_target));
    }

    #[test]
    fn save_and_plot_formats_grade_every_branch() {
        assert_eq!(save_format("dxf"), SaveFormat::Dxf);
        assert_eq!(save_format("dwg"), SaveFormat::Dwg);
        assert_eq!(save_format(""), SaveFormat::NoExtension);
        assert_eq!(save_format("txt"), SaveFormat::Unknown("txt".into()));

        assert_eq!(plot_format("svg"), PlotFormat::Svg);
        assert_eq!(plot_format("pdf"), PlotFormat::Pdf);
        assert_eq!(plot_format("png"), PlotFormat::Png);
        assert_eq!(plot_format(""), PlotFormat::NoExtension);
        assert_eq!(plot_format("txt"), PlotFormat::Unknown("txt".into()));
    }
}
