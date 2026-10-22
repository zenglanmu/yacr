//! Launch/CLI option parsing for the desktop hosts (pure functions).
//!
//! The parse is deliberately pure: [`LaunchOptions::parse_args`] returns the
//! parsed options **and** the diagnostics it would emit, so every rule is
//! unit-testable without a process, a display or a filesystem. The concrete host
//! prints them ([`LaunchOptions::from_args`]) and owns the platform-specific
//! window policy (maximisation, config paths) itself.

use std::path::PathBuf;

/// Default initial window width in physical pixels.
pub const INITIAL_WIDTH: u32 = 960;
/// Default initial window height in physical pixels.
pub const INITIAL_HEIGHT: u32 = 600;

/// CLI/startup options for a desktop host (owned by the main thread).
///
/// `--open <path>` (or `YACR_TEST_DWG`) is the real fixture funnel: the path is
/// read through the same `PlatformFiles` open path, never a fake headless open.
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// A drawing to open at startup (real file, read through the open funnel).
    pub drawing: Option<PathBuf>,
    /// Explicit CAD font directory (`--fonts-dir`).
    pub fonts_dir: Option<PathBuf>,
    /// Explicit raster-image directory (`--images-dir`).
    pub images_dir: Option<PathBuf>,
    /// Initial logical window size.
    pub size: [f64; 2],
    /// `--probe` / `YACR_PROBE_ONLY`: keep the old probe marker path and report
    /// CAD as unsupported (no CAD session).
    pub probe_only: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            drawing: None,
            fonts_dir: None,
            images_dir: None,
            size: [f64::from(INITIAL_WIDTH), f64::from(INITIAL_HEIGHT)],
            probe_only: false,
        }
    }
}

impl LaunchOptions {
    /// Parse CLI arguments (the real fixture funnel; never a fake open).
    ///
    /// Recognised: `--open <path>`, `--fonts-dir <path>`, `--images-dir <path>`
    /// and `--probe`. A **bare positional** argument (does not start with `-`) is
    /// a drawing path, exactly as if passed to `--open`, so a desktop entry can
    /// launch `yacr <drawing>`. When both are present `--open` wins. Only the
    /// first positional is opened (multi-document open is NOT IMPLEMENTED); every
    /// dropped or unsupported argument is reported on stderr instead of being
    /// silently ignored. Parsing is pure: the returned [`LaunchOptions`] and the
    /// diagnostics are unit-testable, and [`from_args`] emits them verbatim.
    ///
    /// [`from_args`]: LaunchOptions::from_args
    pub fn parse_args<I: Iterator<Item = String>>(args: I) -> (Self, Vec<String>) {
        let mut options = Self::default();
        let mut diagnostics = Vec::new();
        let mut positionals: Vec<String> = Vec::new();
        let mut args = args;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--open" => options.drawing = args.next().map(PathBuf::from),
                "--fonts-dir" => options.fonts_dir = args.next().map(PathBuf::from),
                "--images-dir" => options.images_dir = args.next().map(PathBuf::from),
                "--probe" => options.probe_only = true,
                _ => {
                    if arg.starts_with('-') {
                        // Unknown flags stay ignored exactly as before: never a
                        // path and never a new failure.
                    } else if arg.contains("://") {
                        // A URI scheme (e.g. `file://`) is NOT a filesystem path.
                        // Explicit refusal, never a silent mis-parse.
                        diagnostics.push(format!(
                            "error: unsupported argument `{arg}` (URI scheme is NOT IMPLEMENTED; pass a plain filesystem path or use --open <path>)"
                        ));
                    } else {
                        positionals.push(arg);
                    }
                }
            }
        }

        if let Some((first, extras)) = positionals.split_first() {
            if options.drawing.is_some() {
                diagnostics.push(format!(
                    "[probe] --open takes precedence; positional drawing argument ignored: {first}"
                ));
            } else {
                diagnostics.push(format!("[probe] positional drawing argument: {first}"));
                options.drawing = Some(PathBuf::from(first));
            }
            if !extras.is_empty() {
                diagnostics.push(format!(
                    "[probe] extra positional argument(s) ignored (multi-document open is NOT IMPLEMENTED): {}",
                    extras.join(" ")
                ));
            }
        }

        (options, diagnostics)
    }

    /// [`LaunchOptions::parse_args`] plus emitting each diagnostic verbatim on
    /// stderr (English lines; the `[probe]`/`error:` prefixes match the existing
    /// style and stay greppable).
    pub fn from_args<I: Iterator<Item = String>>(args: I) -> Self {
        let (options, diagnostics) = Self::parse_args(args);
        for diagnostic in &diagnostics {
            eprintln!("{diagnostic}");
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn launch_options_parse_cli() {
        let opts = LaunchOptions::from_args(
            [
                "--open",
                "/tmp/a.dwg",
                "--fonts-dir",
                "/tmp/fonts",
                "--images-dir",
                "/tmp/img",
                "--probe",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/a.dwg")));
        assert_eq!(opts.fonts_dir.as_deref(), Some(Path::new("/tmp/fonts")));
        assert_eq!(opts.images_dir.as_deref(), Some(Path::new("/tmp/img")));
        assert!(opts.probe_only);
        assert_eq!(opts.size, [960.0, 600.0]);
        // Unknown args are ignored; missing values leave the field unset.
        let empty = LaunchOptions::from_args(["--nope"].into_iter().map(String::from));
        assert!(empty.drawing.is_none() && !empty.probe_only);
    }

    #[test]
    fn positional_argument_opens_the_drawing() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["/tmp/plan.dwg"].into_iter().map(String::from));
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/plan.dwg")));
        assert_eq!(
            diagnostics,
            vec!["[probe] positional drawing argument: /tmp/plan.dwg".to_string()]
        );
    }

    #[test]
    fn open_flag_takes_precedence_over_positional() {
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["--open", "/tmp/real.dwg", "/tmp/ignored.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/real.dwg")));
        assert_eq!(
            diagnostics,
            vec![
                "[probe] --open takes precedence; positional drawing argument ignored: /tmp/ignored.dwg"
                    .to_string()
            ]
        );

        // `--open` after the positional also wins (precedence is not order-based).
        let (opts, _) = LaunchOptions::parse_args(
            ["/tmp/ignored.dwg", "--open", "/tmp/real.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/real.dwg")));
    }

    #[test]
    fn multiple_positionals_open_first_and_report_extras() {
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["/tmp/first.dwg", "/tmp/second.dwg", "/tmp/third.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/first.dwg")));
        assert_eq!(
            diagnostics,
            vec![
                "[probe] positional drawing argument: /tmp/first.dwg".to_string(),
                "[probe] extra positional argument(s) ignored (multi-document open is NOT IMPLEMENTED): /tmp/second.dwg /tmp/third.dwg"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn file_uri_is_diagnosed_unsupported_and_not_a_path() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["file:///tmp/plan.dwg"].into_iter().map(String::from));
        assert!(opts.drawing.is_none(), "a URI must never become a path");
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].starts_with("error: "));
        assert!(diagnostics[0].contains("file:///tmp/plan.dwg"));
        assert!(diagnostics[0].contains("NOT IMPLEMENTED"));

        // A URI alongside a real positional: the refusal is reported and the
        // plain path still opens.
        let (opts, diagnostics) = LaunchOptions::parse_args(
            ["file:///tmp/a.dwg", "/tmp/b.dwg"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/b.dwg")));
        assert!(diagnostics.iter().any(|line| line.starts_with("error: ")));
        assert!(diagnostics
            .iter()
            .any(|line| line == "[probe] positional drawing argument: /tmp/b.dwg"));
    }

    #[test]
    fn unknown_flag_still_ignored_and_no_args_yields_none() {
        let (opts, diagnostics) =
            LaunchOptions::parse_args(["--frobnicate", "-x"].into_iter().map(String::from));
        assert!(opts.drawing.is_none());
        assert!(diagnostics.is_empty(), "unknown flags are silently ignored");

        let (opts, diagnostics) = LaunchOptions::parse_args(std::iter::empty::<String>());
        assert!(opts.drawing.is_none());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn positional_does_not_disturb_dir_options() {
        let (opts, _) = LaunchOptions::parse_args(
            [
                "--fonts-dir",
                "/tmp/fonts",
                "/tmp/plan.dwg",
                "--images-dir",
                "/tmp/img",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(opts.drawing.as_deref(), Some(Path::new("/tmp/plan.dwg")));
        assert_eq!(opts.fonts_dir.as_deref(), Some(Path::new("/tmp/fonts")));
        assert_eq!(opts.images_dir.as_deref(), Some(Path::new("/tmp/img")));
    }
}
