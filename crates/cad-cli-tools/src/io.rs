//! io module.

use super::*;

/// Write `contents` to `path` atomically.
///
/// The bytes go to a uniquely named temp file in the same directory, are
/// flushed and synced, and only then replace `path` with a rename. A failure at
/// any step removes the temp file and leaves any existing `path` untouched, so
/// a failed run never leaves a partial result file (audit B30).
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let unique = format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        unique_counter()
    );
    let temp_path = match directory {
        Some(dir) => dir.join(unique),
        None => PathBuf::from(unique),
    };

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

/// Monotonic counter so concurrent CLI invocations never share a temp name.
pub(crate) fn unique_counter() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Document loading
// ---------------------------------------------------------------------------

pub(crate) fn read_input(path: &Path) -> CadResult<Arc<[u8]>> {
    let bytes =
        std::fs::read(path).map_err(|e| CadError::InvalidInput(format!("read failed: {e}")))?;
    Ok(Arc::from(bytes.into_boxed_slice()))
}

pub(crate) fn load_document(invocation: &CliInvocation) -> CadResult<HostController> {
    let bytes = read_input(&invocation.input)?;
    let mut controller = HostController::with_demo_document([1920.0, 1080.0])?;
    controller.open_bytes_with_xref(
        bytes,
        &invocation.input.display().to_string(),
        xref_options_for(&invocation.input),
    )?;
    Ok(controller)
}

/// External-reference policy for a headless open: the input file's own parent
/// directory is the only allowed root, so a relative reference next to the
/// drawing resolves while absolute paths elsewhere stay refused.
#[cfg(not(target_arch = "wasm32"))]
const XREF_TOTAL_BYTES_BUDGET: u64 = 1024 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
fn xref_options_for(input: &Path) -> Option<Arc<XrefOptions>> {
    // A bare file name has no `parent()` (`Path::parent` yields `""`); its
    // sibling xrefs live in the current directory, not nowhere. Resolve the
    // directory to an absolute path so the lexically checked reference does not
    // get re-joined under this root by the carrier.
    let parent = input.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = parent.unwrap_or_else(|| Path::new("."));
    if !dir.is_dir() {
        return None;
    }
    let dir = std::path::absolute(dir).ok()?;
    let source: Arc<dyn cad_resources::XrefFileSource> =
        Arc::new(cad_platform::FsXrefSource::new(vec![dir.clone()]));
    Some(Arc::new(XrefOptions {
        source,
        drawing_dir: dir.clone(),
        roots: vec![dir],
        max_depth: cad_resources::ResourceLimits::default().max_xref_depth,
        total_bytes_budget: XREF_TOTAL_BYTES_BUDGET,
    }))
}

#[cfg(target_arch = "wasm32")]
fn xref_options_for(_input: &Path) -> Option<Arc<XrefOptions>> {
    None
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use cad_resources::XrefPathVerdict;

    #[test]
    fn bare_relative_input_resolves_sibling_xrefs_against_an_absolute_dir() {
        let options = xref_options_for(Path::new("drawing.dxf"))
            .expect("a bare file name still has a drawing directory");
        let expected = std::path::absolute(".").expect("current directory");
        assert!(options.drawing_dir.is_absolute());
        assert_eq!(options.drawing_dir, expected);
        assert_eq!(options.roots, vec![expected]);
    }

    #[test]
    fn relative_directory_input_is_absolute_and_not_double_joined() {
        // A real relative directory under `target/` (already ignored), so this
        // exercises the same shape as `cargo run --open relative/drawing.dxf`.
        let relative = PathBuf::from("target").join(format!(
            "yacr-cli-xref-{}-{}",
            std::process::id(),
            unique_counter()
        ));
        let absolute = std::env::current_dir()
            .expect("current dir")
            .join(&relative);
        std::fs::create_dir_all(&absolute).expect("create relative dir");
        let input = relative.join("drawing.dxf");

        let options = xref_options_for(&input).expect("relative directory input");
        assert!(options.drawing_dir.is_absolute());
        assert_eq!(options.drawing_dir, absolute);
        assert_eq!(options.roots, vec![absolute.clone()]);

        // The lexically checked reference must already contain the root; a
        // carrier that re-joins it would look under `root/root/base.dxf`.
        match cad_resources::check_xref_path("base.dxf", &options.drawing_dir, &options.roots) {
            XrefPathVerdict::Allowed { normalized } => {
                assert_eq!(normalized, absolute.join("base.dxf"));
                assert!(!normalized.to_string_lossy().contains("target/target"));
            }
            XrefPathVerdict::Refused { reason } => panic!("sibling xref refused: {reason}"),
        }

        std::fs::remove_dir_all(&absolute).ok();
    }
}

// ---------------------------------------------------------------------------
// Shared JSON helpers
// ---------------------------------------------------------------------------
