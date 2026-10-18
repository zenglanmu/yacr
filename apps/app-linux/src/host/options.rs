//! Launch/CLI options and the window/config defaults (host startup surface).

use std::path::PathBuf;
use std::time::Duration;

/// Label of the top-level probe window.
pub const WINDOW_LABEL: &str = "probe-window";
/// Label of the child webview that hosts the React frontend.
pub const WEBVIEW_LABEL: &str = "probe-webview";
/// Initial window size in physical pixels.
pub const INITIAL_WIDTH: u32 = 960;
/// Initial window size in physical pixels.
pub const INITIAL_HEIGHT: u32 = 600;
/// Minimum interval between presented frames (bounds CPU/GPU work to ~60 fps).
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// Fixed animation step applied to the marker on frames without input.
pub const ANIMATION_STEP: f32 = 0.01;

/// CLI/startup options for the React host (owned by the main thread).
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

/// Resolve the durable config/preference paths (`$XDG_CONFIG_HOME` or `~/.config`).
///
/// `None` when no home is resolvable: the store then refuses persistence
/// explicitly (the host falls back to in-memory defaults).
pub(super) fn config_paths() -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    match dir {
        Some(dir) => {
            let app = dir.join("yacr");
            (
                Some(app.join("config.json")),
                Some(app.join("preference.json")),
            )
        }
        None => (None, None),
    }
}

/// Whether the window starts maximized.
///
/// Default is maximized; the acceptance-only `YACR_PROBE_NO_MAXIMIZE` env opts
/// out (its presence, not its value, is passed here).
pub(super) fn start_maximized(no_maximize_env_present: bool) -> bool {
    !no_maximize_env_present
}
