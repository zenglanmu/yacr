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
