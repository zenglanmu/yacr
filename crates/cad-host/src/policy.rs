//! Pure host policy: command routing, status ownership and export-format
//! classification. Every function here is free of UI/GPU/platform state, so
//! it is unit-testable in isolation.
use cad_app::CommandId;

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

/// Classification of a lossy-save target by extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFormat {
    Dxf,
    /// `.dwg` output is not verified yet: an explicit deferral.
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
