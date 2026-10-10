//! run module.

use super::*;

/// Execute one CLI operation and produce the pretty-printed JSON document.
///
/// The returned error is structured and carries a stable machine `code`.
pub fn run(invocation: &CliInvocation) -> Result<String, CliError> {
    if matches!(
        invocation.operation,
        CliOperation::FixedViewportRender | CliOperation::Plot | CliOperation::Benchmark
    ) {
        // The browser has no headless device to own, and this CLI host does not
        // own the file/IO path for plotting either: it keeps the explicit
        // "unsupported" answer (the host canvas supplies the device instead).
        // The CPU-only vector plot is native-only too until the wasm CLI host
        // gains its own document/write path; claiming otherwise here would be a
        // fake success.
        #[cfg(target_arch = "wasm32")]
        return Err(cli_error_from_domain(CadError::Unsupported(
            "fixed-viewport rendering, plotting and benchmarking require a native \
             host; wasm receives its device from the host canvas"
                .into(),
        )));
        // Native: the CPU-only vector plot never owns a device, so it runs here
        // rather than in the GPU branch; `png` drives the real headless renderer.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let value = match invocation.operation {
                CliOperation::FixedViewportRender => domain(run_render(invocation))?,
                CliOperation::Plot => domain(run_plot(invocation))?,
                CliOperation::Benchmark => {
                    let controller = domain(load_document(invocation))?;
                    let fonts =
                        domain(load_fonts(&invocation.fonts, &fonts_requested(&controller)))?;
                    domain(run_benchmark(&controller, invocation, fonts.as_ref()))?
                }
                _ => unreachable!(),
            };
            return serde_json::to_string_pretty(&value).map_err(|e| {
                CliError::new(error_code::INVARIANT, format!("cli encode failed: {e}"))
            });
        }
    }
    let mut controller = domain(load_document(invocation))?;
    let value = match invocation.operation {
        CliOperation::Scan => domain(run_scan(&controller))?,
        CliOperation::ProxyReport => domain(run_proxy_report(&controller))?,
        CliOperation::Measure => domain(run_measure(&mut controller, invocation))?,
        CliOperation::BuildRepresentation => {
            // A structural report must be host-font independent: shape text only
            // when the caller passes explicit `--font` entries, never the auto
            // system-default face. Otherwise every text run is shaped into line
            // geometry and `kind_counts.texts` is always 0, making the report
            // depend on the host's installed fonts.
            let fonts = domain(load_fonts(&invocation.fonts, &[]))?;
            domain(run_build_representation(&controller, fonts.as_ref()))?
        }
        CliOperation::Benchmark => unreachable!(),
        CliOperation::FixedViewportRender | CliOperation::Plot => unreachable!(),
    };
    serde_json::to_string_pretty(&value)
        .map_err(|e| CliError::new(error_code::INVARIANT, format!("cli encode failed: {e}")))
}

/// Run an operation with an explicit `--out` file written atomically.
///
/// On success the JSON is returned. When `out` is `Some`, the bytes are written
/// to that path atomically (temp file + rename) before this returns; stdout
/// containment is the caller's responsibility.
pub fn run_to_output(invocation: &CliInvocation, out: Option<&Path>) -> Result<String, CliError> {
    let json = run(invocation)?;
    if let Some(path) = out {
        write_atomic(path, json.as_bytes()).map_err(|e| {
            CliError::new(
                error_code::OUTPUT_WRITE_FAILED,
                format!("failed to write --out: {}", e),
            )
            .with_context(serde_json::json!({ "path": path.display().to_string() }))
        })?;
    }
    Ok(json)
}
