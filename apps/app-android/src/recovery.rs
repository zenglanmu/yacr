//! recovery module.

use super::*;

impl AndroidRecovery {
    pub fn new(directory: impl Into<std::path::PathBuf>) -> Self {
        AndroidRecovery {
            directory: directory.into(),
        }
    }

    /// Deterministic per-document file path, so a bookmarked id maps to one file.
    fn path_for(&self, document: DocumentId) -> std::path::PathBuf {
        self.directory
            .join(format!("yacr-recovery-{:032x}.json", document.0))
    }
}

impl Persistence for AndroidRecovery {
    fn save_recovery(&self, document: DocumentId, bytes: Arc<[u8]>) -> HostFuture<'_, ()> {
        let path = self.path_for(document);
        Box::pin(async move {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CadError::ResourceMissing(format!(
                        "cannot create recovery dir {}: {e}",
                        parent.display()
                    ))
                })?;
            }
            std::fs::write(&path, &bytes).map_err(|e| {
                CadError::ResourceMissing(format!(
                    "cannot write recovery snapshot {}: {e}",
                    path.display()
                ))
            })
        })
    }

    fn load_recovery(&self, document: DocumentId) -> HostFuture<'_, Option<Arc<[u8]>>> {
        let path = self.path_for(document);
        Box::pin(async move {
            match std::fs::read(&path) {
                Ok(bytes) => Ok(Some(Arc::from(bytes.into_boxed_slice()))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(CadError::ResourceMissing(format!(
                    "cannot read recovery snapshot {}: {e}",
                    path.display()
                ))),
            }
        })
    }

    fn discard_recovery_after_confirmation(&self, document: DocumentId) -> HostFuture<'_, ()> {
        let path = self.path_for(document);
        Box::pin(async move {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(CadError::ResourceMissing(format!(
                    "cannot remove recovery snapshot {}: {e}",
                    path.display()
                ))),
            }
        })
    }
}

/// The host's recovery store, if one is configured and enabled.
pub(crate) fn recovery_store(configuration: &AndroidHostConfiguration) -> Option<AndroidRecovery> {
    if !configuration.recovery_enabled {
        return None;
    }
    configuration
        .recovery_directory
        .as_ref()
        .map(AndroidRecovery::new)
}

/// Write the annotation sidecar JSON into a host directory.
///
/// Returns whether the durable write succeeded. A missing directory or any I/O
/// error returns `false`, so the caller never confirms an unsaved export.
pub(crate) fn write_annotation_export(directory: Option<&str>, json: &str) -> bool {
    let Some(directory) = directory else {
        return false;
    };
    let directory = std::path::Path::new(directory);
    if std::fs::create_dir_all(directory).is_err() {
        return false;
    }
    std::fs::write(directory.join("annotations.cadnotes.json"), json.as_bytes()).is_ok()
}
