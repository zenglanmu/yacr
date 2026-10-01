//! yacr headless CLI (spec v2.0 §19).
//!
//! Every operation goes through the same application/database path as the UI
//! and prints structured JSON on stdout. Errors are printed on stderr and exit
//! with a non-zero status; there is no opaque success string.

use std::process::ExitCode;

use cad_cli_tools::{CliInvocation, CliOperation};
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
  render               fixed-viewport GPU frame (requires a GPU environment)
  benchmark            representation build timing for the input

Options:
  --notes <file>          annotation sidecar path (import/export)
  --points \"x,y;x,y;...\"  measurement points in drawing units
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
        eprintln!("unknown operation: {}", arguments[0]);
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut input: Option<String> = None;
    let mut invocation = CliInvocation::new(operation, "");
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--notes" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("--notes needs a path");
                    return ExitCode::from(2);
                };
                invocation.notes = Some(value.into());
            }
            "--points" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("--points needs a value");
                    return ExitCode::from(2);
                };
                match parse_points(value) {
                    Ok(points) => invocation.points = points,
                    Err(error) => {
                        eprintln!("{error}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--allow-fingerprint-mismatch" => invocation.allow_fingerprint_mismatch = true,
            "--font" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    eprintln!("--font needs <name=path> or <path>");
                    return ExitCode::from(2);
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
                eprintln!("unknown option: {other}");
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
            other => {
                if input.is_some() {
                    eprintln!("unexpected extra argument: {other}");
                    return ExitCode::from(2);
                }
                input = Some(other.to_string());
            }
        }
        index += 1;
    }
    let Some(input) = input else {
        eprintln!("missing input drawing path");
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    invocation.input = input.into();

    match cad_cli_tools::run(&invocation) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
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
