//! Desktop implementations of the `cad-platform` host contracts.
//!
//! The file chooser (ashpd XDG portal on Linux, `rfd` on Windows/macOS) and the
//! drawing read run on worker threads, delivering results as
//! [`FileEvent`]s; the config store resolves the XDG/APPDATA/Application-Support
//! paths and writes atomically; the surface source reads the live window.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use cad_domain::{CadError, CadResult};
use cad_platform::{
    ConfigLoad, ConfigStore, FileEvent, FilePurpose, FileTarget, PlatformFiles, SavePurpose,
    SurfaceMetrics, SurfaceSource,
};
use cad_ui_slint::{MessageSource, UiHandle};

use super::{atomic_write, FilePicker, LinuxOptions, SavePathProvider};

/// Desktop file selection/read/write.
pub struct DesktopFiles {
    headless: bool,
    drawing: Option<PathBuf>,
    locale: String,
    picker: FilePicker,
    saver: SavePathProvider,
    events: Arc<Mutex<Vec<FileEvent>>>,
    /// The successfully-opened source path (armed by the core only on success).
    source: Arc<Mutex<Option<PathBuf>>>,
}

impl DesktopFiles {
    pub fn new(options: &LinuxOptions, picker: FilePicker, saver: SavePathProvider) -> Self {
        Self {
            headless: options.headless,
            drawing: options.drawing.clone(),
            locale: options.locale.clone(),
            picker,
            saver,
            events: Arc::new(Mutex::new(Vec::new())),
            source: Arc::new(Mutex::new(None)),
        }
    }
}

impl PlatformFiles for DesktopFiles {
    fn begin_open(&self) -> CadResult<()> {
        if self.headless {
            let Some(drawing) = &self.drawing else {
                return Err(CadError::Unsupported(
                    MessageSource::from_request(&self.locale).text("linux.open_path", &[]),
                ));
            };
            let target = FileTarget::from_path(&drawing.to_string_lossy());
            return self.begin_open_target(&target);
        }
        let events = self.events.clone();
        let picker = self.picker.clone();
        std::thread::Builder::new()
            .name("linux-file-picker".into())
            .spawn(move || {
                let event = match picker() {
                    Ok(path) => match std::fs::read(&path) {
                        Ok(bytes) => FileEvent::Opened {
                            target: FileTarget::from_path(&path.to_string_lossy()),
                            bytes: Arc::from(bytes),
                        },
                        Err(error) => FileEvent::Failed {
                            purpose: FilePurpose::Open,
                            reason: open_read_failure(&path, error),
                        },
                    },
                    Err(CadError::Cancelled) => FileEvent::Cancelled {
                        purpose: FilePurpose::Open,
                    },
                    Err(error) => FileEvent::Failed {
                        purpose: FilePurpose::Open,
                        reason: error.to_string(),
                    },
                };
                events.lock().unwrap().push(event);
            })
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        Ok(())
    }

    fn begin_open_target(&self, target: &FileTarget) -> CadResult<()> {
        let path = PathBuf::from(&target.opaque);
        // A missing path is refused up front rather than falling back to the
        // synthetic demo, whether the read happens here or on the worker.
        if !path.exists() {
            return Err(CadError::InvalidInput(format!(
                "drawing path does not exist: {}",
                path.display()
            )));
        }
        if self.headless {
            let bytes = std::fs::read(&path)
                .map_err(|error| CadError::InvalidInput(open_read_failure(&path, error)))?;
            self.events.lock().unwrap().push(FileEvent::Opened {
                target: FileTarget::from_path(&path.to_string_lossy()),
                bytes: Arc::from(bytes),
            });
            return Ok(());
        }
        let events = self.events.clone();
        std::thread::Builder::new()
            .name("linux-drawing-read".into())
            .spawn(move || {
                let event = match std::fs::read(&path) {
                    Ok(bytes) => FileEvent::Opened {
                        target: FileTarget::from_path(&path.to_string_lossy()),
                        bytes: Arc::from(bytes),
                    },
                    Err(error) => FileEvent::Failed {
                        purpose: FilePurpose::Open,
                        reason: open_read_failure(&path, error),
                    },
                };
                events.lock().unwrap().push(event);
            })
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        Ok(())
    }

    fn begin_save(&self, purpose: SavePurpose, default_name: &str) -> CadResult<()> {
        let events = self.events.clone();
        let saver = self.saver.clone();
        let name = default_name.to_string();
        std::thread::Builder::new()
            .name("linux-save-dialog".into())
            .spawn(move || {
                let event = match saver(&name) {
                    Ok(path) => FileEvent::SaveTarget {
                        purpose,
                        target: FileTarget::from_path(&path.to_string_lossy()),
                    },
                    Err(CadError::Cancelled) => FileEvent::Cancelled {
                        purpose: FilePurpose::Save(purpose),
                    },
                    Err(error) => FileEvent::Failed {
                        purpose: FilePurpose::Save(purpose),
                        reason: error.to_string(),
                    },
                };
                events.lock().unwrap().push(event);
            })
            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
        Ok(())
    }

    fn write_atomic(&self, target: &FileTarget, bytes: &[u8]) -> CadResult<()> {
        atomic_write(Path::new(&target.opaque), bytes)
    }

    fn refuses_source_overwrite(&self, target: &FileTarget) -> bool {
        let source = self.source.lock().unwrap().clone();
        match source {
            Some(source) => same_file(&source, Path::new(&target.opaque)),
            None => false,
        }
    }

    fn set_source(&self, target: Option<&FileTarget>) {
        *self.source.lock().unwrap() = target.map(|target| PathBuf::from(&target.opaque));
    }

    fn poll(&self) -> Vec<FileEvent> {
        let mut events = self.events.lock().unwrap();
        std::mem::take(&mut *events)
    }
}

/// Whether `target` is the opened source file, so the save refuses to overwrite
/// the file being read.
///
/// On Unix the comparison uses filesystem identity (device + inode) when both
/// paths exist: a case-insensitive mount, a symlink or a bind mount that a
/// path-string comparison would miss is still caught. A not-yet-existing target
/// cannot be the existing source file, so the fallback path comparison is only
/// for the degenerate case.
fn same_file(source: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(source_meta), Ok(target_meta)) =
            (std::fs::metadata(source), std::fs::metadata(target))
        {
            return source_meta.dev() == target_meta.dev()
                && source_meta.ino() == target_meta.ino();
        }
    }
    let canonical = |candidate: &Path| std::fs::canonicalize(candidate).ok();
    match (canonical(source), canonical(target)) {
        (Some(source), Some(target)) => source == target,
        _ => source == target,
    }
}

/// A localized reason for a failed drawing read.
///
/// A missing file gets the explicit "drawing path does not exist" message (the
/// picker can return a path that vanished before the read); anything else keeps
/// the raw io error. The core shows this reason verbatim, so it must not be
/// pre-wrapped in an error `Debug`.
fn open_read_failure(path: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        format!("drawing path does not exist: {}", path.display())
    } else {
        error.to_string()
    }
}

/// Desktop config/preference persistence.
pub struct DesktopConfigStore {
    config_path: Option<PathBuf>,
    preference_path: Option<PathBuf>,
}

impl DesktopConfigStore {
    pub fn new(config_path: Option<PathBuf>, preference_path: Option<PathBuf>) -> Self {
        Self {
            config_path,
            preference_path,
        }
    }
}

impl ConfigStore for DesktopConfigStore {
    fn load_config(&self) -> ConfigLoad {
        load_file(self.config_path.as_deref())
    }

    fn load_preference(&self) -> ConfigLoad {
        load_file(self.preference_path.as_deref())
    }

    fn persist_preference(&self, text: &str) -> CadResult<()> {
        match &self.preference_path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)
                            .map_err(|error| CadError::InvalidInput(error.to_string()))?;
                    }
                }
                atomic_write(path, text.as_bytes())
            }
            // No durable location was resolvable; the patch is still applied in
            // memory and the caller can report the missing path.
            None => Ok(()),
        }
    }
}

fn load_file(path: Option<&Path>) -> ConfigLoad {
    let Some(path) = path else {
        return ConfigLoad::Absent;
    };
    match std::fs::read_to_string(path) {
        Ok(text) => ConfigLoad::Text {
            path: path.display().to_string(),
            text,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigLoad::Absent,
        Err(error) => ConfigLoad::Unreadable {
            path: path.display().to_string(),
            reason: error.to_string(),
        },
    }
}

/// Desktop surface metrics: the live window's CAD rectangle and scale, with
/// explicit zero safe-area insets (the desktop has no notch/system-bar API).
pub struct DesktopSurface {
    handle: std::rc::Rc<std::cell::RefCell<Option<UiHandle>>>,
}

impl DesktopSurface {
    pub fn new(handle: std::rc::Rc<std::cell::RefCell<Option<UiHandle>>>) -> Self {
        Self { handle }
    }
}

impl SurfaceSource for DesktopSurface {
    fn surface(&self) -> Option<SurfaceMetrics> {
        let handle = self.handle.borrow();
        let handle = handle.as_ref()?;
        let (logical, scale) = handle.cad_surface_size()?;
        Some(SurfaceMetrics {
            logical,
            scale,
            safe_insets: [0.0; 4],
        })
    }
}
