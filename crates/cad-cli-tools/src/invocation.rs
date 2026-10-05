//! invocation module.

use super::*;

/// Machine schema version for CLI result and error documents.
///
/// This is a stable contract identifier: it does not change with `--locale`.
pub const CLI_SCHEMA_VERSION: u32 = 1;

/// Default locale for human-facing messages (simplified Chinese).
pub const DEFAULT_LOCALE: &str = "zh-CN";

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOperation {
    Scan,
    ProxyReport,
    Measure,
    BuildRepresentation,
    FixedViewportRender,
    Plot,
    Benchmark,
}

impl CliOperation {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "scan" => Some(Self::Scan),
            "proxy-report" => Some(Self::ProxyReport),
            "measure" => Some(Self::Measure),
            "build-representation" => Some(Self::BuildRepresentation),
            "render" => Some(Self::FixedViewportRender),
            "plot" => Some(Self::Plot),
            "benchmark" => Some(Self::Benchmark),
            _ => None,
        }
    }

    /// The stable machine key reported in result and error documents.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::ProxyReport => "proxy-report",
            Self::Measure => "measure",
            Self::BuildRepresentation => "build-representation",
            Self::FixedViewportRender => "render",
            Self::Plot => "plot",
            Self::Benchmark => "benchmark",
        }
    }
}

// ---------------------------------------------------------------------------
// Human locale (never affects machine output)
// ---------------------------------------------------------------------------

/// Locale for human-facing strings printed on stderr.
///
/// Machine output (stdout JSON and error `code`s) is locale-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    ZhCn,
    En,
}

impl Locale {
    /// Normalize an input tag; unsupported tags fall back to `zh-CN`.
    pub fn parse(tag: &str) -> Self {
        let lower = tag.trim().to_ascii_lowercase();
        if lower == "en" || lower.starts_with("en-") {
            Locale::En
        } else {
            Locale::ZhCn
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Locale::ZhCn => "zh-CN",
            Locale::En => "en",
        }
    }
}

// ---------------------------------------------------------------------------
// Plot output format
// ---------------------------------------------------------------------------

/// Output format for the `plot` operation.
///
/// `Png` is the raster path (headless GPU + PNG encode, the default). `Svg` and
/// `Pdf` are the CPU-only vector path: they never create a GPU device, so they
/// work on a machine with no Vulkan adapter at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotFormat {
    #[default]
    Png,
    Svg,
    Pdf,
}

impl PlotFormat {
    /// Parse a user-supplied `--plot-format` value. Unknown values stay `None`
    /// so the caller can report a usage error instead of guessing.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "svg" => Some(Self::Svg),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// The stable machine key (extension-friendly).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
        }
    }

    /// True for the CPU-only vector formats, which must not touch a GPU.
    pub fn is_vector(self) -> bool {
        matches!(self, Self::Svg | Self::Pdf)
    }
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// A versioned invocation record; re-running it reproduces the command.
#[derive(Debug, Clone)]
pub struct CliInvocation {
    pub schema_version: u32,
    pub operation: CliOperation,
    pub input: PathBuf,
    /// Measure input in the input file's coordinate space.
    pub points: Vec<Point3>,
    /// Fonts to shape text with, as `(key, path)`.
    pub fonts: Vec<(String, PathBuf)>,
    /// Optional file that receives the JSON result atomically.
    pub out: Option<PathBuf>,
    /// Optional PNG that `render` writes the offscreen frame to.
    pub png: Option<PathBuf>,
    /// Offscreen frame width in pixels for `render`/`plot`.
    pub render_width: u32,
    /// Offscreen frame height in pixels for `render`/`plot`.
    pub render_height: u32,
    /// Layout to plot by name (`plot`). `None` selects the first layout.
    pub layout: Option<String>,
    /// Plot resolution in dots per inch (`plot`). When set, the canvas is sized
    /// from the sheet; when absent the `--width`/`--height` canvas is used.
    pub plot_dpi: Option<f64>,
    /// Plot output format (`plot`). `Png` (default) runs the headless GPU
    /// raster path; `Svg`/`Pdf` run the CPU-only vector path and never create a
    /// GPU device.
    pub plot_format: PlotFormat,
    /// Locale for human-facing stderr messages (never machine output).
    pub locale: Locale,
}

/// Default offscreen frame width for `render`/`plot`.
pub const DEFAULT_RENDER_WIDTH: u32 = 1280;
/// Default offscreen frame height for `render`/`plot`.
pub const DEFAULT_RENDER_HEIGHT: u32 = 720;

impl CliInvocation {
    pub fn new(operation: CliOperation, input: impl Into<PathBuf>) -> Self {
        CliInvocation {
            schema_version: CLI_SCHEMA_VERSION,
            operation,
            input: input.into(),
            points: Vec::new(),
            fonts: Vec::new(),
            out: None,
            png: None,
            render_width: DEFAULT_RENDER_WIDTH,
            render_height: DEFAULT_RENDER_HEIGHT,
            layout: None,
            plot_dpi: None,
            plot_format: PlotFormat::default(),
            locale: Locale::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Structured errors
// ---------------------------------------------------------------------------

/// Stable machine error codes. These never change with locale.
pub mod error_code {
    pub const INVALID_INPUT: &str = "invalid_input";
    pub const UNSUPPORTED: &str = "unsupported";
    pub const NOT_IMPLEMENTED: &str = "not_implemented";
    pub const RESOURCE_MISSING: &str = "resource_missing";
    pub const CORRUPT_DATA: &str = "corrupt_data";
    pub const GPU_FAILURE: &str = "gpu_failure";
    pub const INVARIANT: &str = "invariant";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const CANCELLED: &str = "cancelled";
    pub const STALE_RESULT: &str = "stale_result";
    /// The operation's input contract was violated (bad option combination).
    pub const USAGE: &str = "usage";
    /// A result could not be written to `--out`.
    pub const OUTPUT_WRITE_FAILED: &str = "output_write_failed";
}

/// A structured CLI failure ready to be serialized to stderr.
///
/// `code` and `context` are stable machine data; `message` is human text whose
/// language may follow `locale`.
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: String,
    pub message: String,
    pub context: serde_json::Value,
    /// Locale for the human-facing `message` line only.
    pub locale: Locale,
}

impl CliError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        CliError {
            code: code.into(),
            message: message.into(),
            context: serde_json::Value::Null,
            locale: Locale::default(),
        }
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    /// Set the human-message locale. Machine keys are unaffected.
    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// A usage error (exit code 2) for bad argument combinations.
    pub fn usage(message: impl Into<String>) -> Self {
        CliError::new(error_code::USAGE, message)
    }

    /// The error document written to stderr.
    ///
    /// `operation` and every key are stable; only `error.message` is localized.
    pub fn to_json(&self, operation: CliOperation) -> serde_json::Value {
        serde_json::json!({
            "schema_version": CLI_SCHEMA_VERSION,
            "operation": operation.as_str(),
            "error": {
                "code": self.code,
                "message": self.message,
                "context": self.context,
            }
        })
    }

    /// Process exit status: 2 for usage errors, 1 for everything else.
    pub fn exit_code(&self) -> u8 {
        if self.code == error_code::USAGE {
            2
        } else {
            1
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

/// Map a domain error to a stable machine code.
pub fn error_code_for(error: &CadError) -> &'static str {
    match error {
        CadError::NotImplemented(_) => error_code::NOT_IMPLEMENTED,
        CadError::InvalidInput(_) => error_code::INVALID_INPUT,
        CadError::Unsupported(_) => error_code::UNSUPPORTED,
        CadError::ResourceMissing(_) => error_code::RESOURCE_MISSING,
        CadError::CorruptData(_) => error_code::CORRUPT_DATA,
        CadError::GpuFailure(_) => error_code::GPU_FAILURE,
        CadError::Invariant(_) => error_code::INVARIANT,
        CadError::PermissionDenied => error_code::PERMISSION_DENIED,
        CadError::Cancelled => error_code::CANCELLED,
        CadError::StaleResult => error_code::STALE_RESULT,
    }
}

/// Convert a domain error into a structured CLI error with redacted context.
///
/// Only the technical error kind is attached as context; file contents, paths
/// and identity values are never copied into the default machine document.
pub fn cli_error_from_domain(error: CadError) -> CliError {
    let code = error_code_for(&error);
    let message = error.to_string();
    CliError::new(code, message)
}

pub(crate) fn domain<T>(result: CadResult<T>) -> Result<T, CliError> {
    result.map_err(cli_error_from_domain)
}

// ---------------------------------------------------------------------------
// Atomic output
// ---------------------------------------------------------------------------
