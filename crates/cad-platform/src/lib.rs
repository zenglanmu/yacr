//! Host contracts use own handles, never platform paths or native objects.
use cad_domain::*;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = CadResult<T>> + 'a>>;
#[derive(Debug, Clone)]
pub struct FileToken(pub String);
pub struct FileGrant {
    pub token: FileToken,
    pub display_name: String,
    pub writable: bool,
    pub persistent: bool,
}
pub trait FileAccess {
    fn pick_drawing(&self) -> HostFuture<'_, FileGrant>;
    fn read(&self, token: &FileToken) -> HostFuture<'_, Arc<[u8]>>;
    fn export_atomic(&self, name_hint: &str, bytes: Arc<[u8]>) -> HostFuture<'_, FileGrant>;
}

/// Host-side font fetching.
///
/// The core only builds catalog URLs (`cad-resources::plan_fonts`); the host
/// performs the actual network/asset access, applies the font licence and
/// caches the bytes before they are registered with the shaping engine.
pub trait FontLoader {
    fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>>;

    /// Fetch the font catalog JSON.
    ///
    /// Defaults to the same byte-fetch path as individual fonts; hosts with a
    /// dedicated catalog source can override it. The catalog URL is derived by
    /// [`fonts::catalog_url`] from the base passed to
    /// [`fonts::load_font_engine`].
    fn load_catalog(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
        self.load_font(url)
    }
}

pub mod fonts;

/// Drive a [`HostFuture`] to completion on a no-op waker.
///
/// This is only sound for host futures whose steps are always ready (asset
/// reads, in-memory caches, localStorage) — the Android asset loader and the
/// tests use it. A future that truly parks (network/GPU) must run on the host's
/// own executor instead; the caller decides which it has.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

pub trait Persistence {
    /// Recovery cache is not a permanent user backup.
    fn save_recovery(&self, document: DocumentId, bytes: Arc<[u8]>) -> HostFuture<'_, ()>;
    fn load_recovery(&self, document: DocumentId) -> HostFuture<'_, Option<Arc<[u8]>>>;
    fn discard_recovery_after_confirmation(&self, document: DocumentId) -> HostFuture<'_, ()>;
}
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
pub struct TaskOutput {
    pub stamp: TaskStamp,
    pub bytes: Arc<[u8]>,
}
pub trait TaskExecutor {
    fn spawn(
        &self,
        stamp: TaskStamp,
        cancel: CancellationToken,
        task: Box<dyn FnOnce() -> CadResult<Arc<[u8]>> + Send>,
    ) -> CadResult<RequestId>;
    fn poll(&self) -> CadResult<Vec<TaskOutput>>;
}
pub trait Clipboard {
    fn copy_text(&self, text: &str) -> HostFuture<'_, ()>;
}
pub enum LifecycleEvent {
    Pause,
    Resume,
    LowMemory,
    SurfaceLost,
    SurfaceReady,
    ExitRequested,
}
pub trait HostLifecycle {
    fn handle(&mut self, event: LifecycleEvent) -> CadResult<()>;
}
pub struct HostTexture {
    pub token: u64,
    pub device_generation: u64,
    pub width: u32,
    pub height: u32,
}
pub trait RenderHost {
    /// One presentation coordinator; no per-frame full image readback.
    fn acquire_target(&mut self) -> CadResult<HostTexture>;
    fn compose_and_present(&mut self, target: HostTexture) -> CadResult<()>;
    fn request_redraw(&self);
}
