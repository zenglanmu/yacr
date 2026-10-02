//! yacr headless CLI (spec v2.0 §19).
//!
//! Every operation goes through the same application/database path as the UI.
//! stdout carries exactly the structured JSON result (or, with `--out`, is
//! empty); all human-facing text and every error goes to stderr. Errors are
//! structured and the process exits non-zero — there is no opaque success.

use std::path::PathBuf;
use std::process::ExitCode;

use cad_cli_tools::{CliError, CliInvocation, CliOperation, Locale};
use cad_domain::Point3;

const USAGE: &str = "\
yacr headless CLI

Usage:
  cad-cli-tools <operation> <input.dwg> [options]

Operations:
  scan                 entity/layer/bounds/completeness summary
  proxy-report         entity capability table and proxy diagnostics
  measure              distance (2 points), angle (3), length (>=4)
  import-notes         import a .cadnotes.json sidecar
  export-notes         export the sidecar (marks saved only after write)
  build-representation primitive/vertex counts through the provider registry
  render               fixed-viewport frame on a headless GPU adapter
  plot                 paper-space layout -> raster PNG (headless GPU)
  benchmark            representation build timing for the input

Options:
  --notes <file>          annotation sidecar path (import/export)
  --points \"x,y;x,y;...\"  measurement points in drawing units
  --out <file>            write the JSON result to <file> atomically
                          (same-dir temp file + rename); stdout stays empty
  --png <file>            render/plot: write the raster as a PNG (atomic)
  --width <u32>           render/plot: frame width in pixels (default 1280)
  --height <u32>          render/plot: frame height in pixels (default 720)
  --layout <name>         plot: layout to plot (default: the first layout)
  --dpi <f64>             plot: raster resolution; sizes the canvas from the
                          sheet instead of --width/--height
  --locale <tag>          human-facing stderr language: zh-CN (default) or en.
                          Machine output keys/schema never change with locale.
  --allow-fingerprint-mismatch  import despite a mismatched drawing hash
  --font <name=path>      register a TTF/OTF/WOFF font for text shaping
                          (repeatable; name defaults to the file name)
";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() || arguments[0] == "--help" || arguments[0] == "-h" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let Some(operation) = CliOperation::parse(&arguments[0]) else {
        let locale = pre_scan_locale(&arguments);
        let error = CliError::usage(format!("unknown operation: {}", arguments[0]));
        let exit = error.exit_code();
        eprint!("{USAGE}");
        report(
            &error.with_locale(locale),
            operation_for_unknown(&arguments[0]),
        );
        return ExitCode::from(exit);
    };

    let mut input: Option<String> = None;
    let mut invocation = CliInvocation::new(operation, "");
    let mut out: Option<PathBuf> = None;
    // Pre-scan `--locale` so a bad option's human message uses the requested
    // language regardless of argument order.
    let locale = pre_scan_locale(&arguments);
    invocation.locale = locale;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--notes" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--notes needs a path"), operation, locale);
                };
                invocation.notes = Some(value.into());
            }
            "--points" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--points needs a value"), operation, locale);
                };
                match parse_points(value) {
                    Ok(points) => invocation.points = points,
                    Err(error) => return fail(CliError::usage(error), operation, locale),
                }
            }
            "--out" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--out needs a path"), operation, locale);
                };
                out = Some(value.into());
            }
            "--png" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--png needs a path"), operation, locale);
                };
                invocation.png = Some(value.into());
            }
            "--width" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(
                        CliError::usage("--width needs a pixel count"),
                        operation,
                        locale,
                    );
                };
                match parse_positive_dimension(value) {
                    Ok(width) => invocation.render_width = width,
                    Err(message) => return fail(CliError::usage(message), operation, locale),
                }
            }
            "--height" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(
                        CliError::usage("--height needs a pixel count"),
                        operation,
                        locale,
                    );
                };
                match parse_positive_dimension(value) {
                    Ok(height) => invocation.render_height = height,
                    Err(message) => return fail(CliError::usage(message), operation, locale),
                }
            }
            "--layout" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--layout needs a name"), operation, locale);
                };
                invocation.layout = Some(value.clone());
            }
            "--dpi" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(
                        CliError::usage("--dpi needs a positive number"),
                        operation,
                        locale,
                    );
                };
                match parse_positive_finite(value) {
                    Ok(dpi) => invocation.plot_dpi = Some(dpi),
                    Err(message) => return fail(CliError::usage(message), operation, locale),
                }
            }
            "--locale" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(CliError::usage("--locale needs a tag"), operation, locale);
                };
                invocation.locale = Locale::parse(value);
            }
            "--allow-fingerprint-mismatch" => invocation.allow_fingerprint_mismatch = true,
            "--font" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return fail(
                        CliError::usage("--font needs <name=path> or <path>"),
                        operation,
                        locale,
                    );
                };
                let (name, path) = match value.split_once('=') {
                    Some((name, path)) => (name.to_string(), path.to_string()),
                    None => {
                        let path = value.clone();
                        let name = std::path::Path::new(&path)
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.clone());
                        (name, path)
                    }
                };
                invocation.fonts.push((name, path.into()));
            }
            other if other.starts_with("--") => {
                return fail(
                    CliError::usage(format!("unknown option: {other}")),
                    operation,
                    locale,
                );
            }
            other => {
                if input.is_some() {
                    return fail(
                        CliError::usage(format!("unexpected extra argument: {other}")),
                        operation,
                        locale,
                    );
                }
                input = Some(other.to_string());
            }
        }
        index += 1;
    }
    let Some(input) = input else {
        return fail(
            CliError::usage("missing input drawing path"),
            operation,
            locale,
        );
    };
    invocation.input = input.into();

    match cad_cli_tools::run_to_output(&invocation, out.as_deref()) {
        Ok(json) => {
            // stdout stays pure JSON: with `--out` it is intentionally empty.
            if out.is_none() {
                println!("{json}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(error.with_locale(locale), operation, locale),
    }
}

/// Find a `--locale <tag>` pair anywhere in the arguments, for early errors.
fn pre_scan_locale(arguments: &[String]) -> Locale {
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == "--locale" {
            if let Some(tag) = arguments.get(index + 1) {
                return Locale::parse(tag);
            }
        }
        index += 1;
    }
    Locale::default()
}

/// Localize the structured error message and print it to stderr, then return
/// the non-zero exit code. The machine `code` is locale-independent.
fn fail(error: CliError, operation: CliOperation, locale: Locale) -> ExitCode {
    let exit = error.exit_code();
    report(&error.with_locale(locale), operation);
    ExitCode::from(exit)
}

fn report(error: &CliError, operation: CliOperation) {
    eprintln!("{}", human_message(error, operation));
    match serde_json::to_string_pretty(&error.to_json(operation)) {
        Ok(json) => eprintln!("{json}"),
        Err(_) => eprintln!(
            "{{\"schema_version\":1,\"operation\":\"{}\"}}",
            operation.as_str()
        ),
    }
}

/// A short human-facing line; the structured document follows on the next line.
///
/// `--locale` only affects this line (and the default zh-CN case), never the
/// stable machine keys or error codes.
fn human_message(error: &CliError, operation: CliOperation) -> String {
    // The locale is echoed in the invocation; the human prefix is intentionally
    // minimal and does not encode any machine-relevant data beyond the code.
    if error.locale() == Locale::En {
        format!(
            "[{}] error {}: {}",
            operation.as_str(),
            error.code,
            error.message
        )
    } else {
        format!(
            "[{}] 错误 {}: {}",
            operation.as_str(),
            error.code,
            error.message
        )
    }
}

/// Unknown operations have no `CliOperation`; report under a stable placeholder.
fn operation_for_unknown(_name: &str) -> CliOperation {
    CliOperation::Scan
}

/// Parse a render dimension; zero and non-numeric values are usage errors
/// (never a silent `max(1)` clamp).
fn parse_positive_dimension(value: &str) -> Result<u32, String> {
    match value.trim().parse::<u32>() {
        Ok(0) | Err(_) => Err(format!(
            "dimension must be a positive integer (got '{value}')"
        )),
        Ok(dimension) => Ok(dimension),
    }
}

/// Parse a positive, finite floating-point option value.
///
/// `0`, negatives, `NaN` and `inf` are usage errors rather than silently
/// clamped, so a bad resolution can never produce a misleading canvas.
fn parse_positive_finite(value: &str) -> Result<f64, String> {
    match value.trim().parse::<f64>() {
        Ok(number) if number.is_finite() && number > 0.0 => Ok(number),
        _ => Err(format!(
            "value must be a positive finite number (got '{value}')"
        )),
    }
}

fn parse_points(value: &str) -> Result<Vec<Point3>, String> {
    let mut points = Vec::new();
    for pair in value.split(';') {
        if pair.trim().is_empty() {
            continue;
        }
        let coordinates: Vec<&str> = pair.split(',').collect();
        if coordinates.len() < 2 || coordinates.len() > 3 {
            return Err(format!("point '{pair}' must be x,y or x,y,z"));
        }
        let mut numbers = [0.0f64; 3];
        for (slot, text) in numbers.iter_mut().zip(coordinates.iter()) {
            *slot = text
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("point '{pair}' is not numeric"))?;
        }
        points.push(Point3 {
            x: numbers[0],
            y: numbers[1],
            z: numbers[2],
        });
    }
    Ok(points)
}
