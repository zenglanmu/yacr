fn main() -> std::process::ExitCode {
    let argument = std::env::args().nth(1);
    if matches!(argument.as_deref(), None | Some("--help") | Some("-h")) {
        println!("yacr: contract scaffold (no DWG/GPU implementation)\nUsage: cad-cli-tools [scan|proxy-report|measure|import-notes|export-notes|build-representation|render|benchmark]\nAll operations currently return NotImplemented.");
        return std::process::ExitCode::SUCCESS;
    }
    let operation = match argument.as_deref() {
        Some("scan") => cad_cli_tools::CliOperation::Scan,
        Some("proxy-report") => cad_cli_tools::CliOperation::ProxyReport,
        Some("measure") => cad_cli_tools::CliOperation::Measure,
        Some("import-notes") => cad_cli_tools::CliOperation::ImportNotes,
        Some("export-notes") => cad_cli_tools::CliOperation::ExportNotes,
        Some("build-representation") => cad_cli_tools::CliOperation::BuildRepresentation,
        Some("render") => cad_cli_tools::CliOperation::FixedViewportRender,
        Some("benchmark") => cad_cli_tools::CliOperation::Benchmark,
        _ => {
            eprintln!("Unknown operation; use --help");
            return std::process::ExitCode::from(2);
        }
    };
    match cad_cli_tools::run_operation(operation, &[]) {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
