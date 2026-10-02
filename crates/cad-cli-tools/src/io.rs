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
    controller.open_bytes(bytes, &invocation.input.display().to_string())?;
    Ok(controller)
}

// ---------------------------------------------------------------------------
// Shared JSON helpers
// ---------------------------------------------------------------------------
