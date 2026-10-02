//! Android host font loading from APK assets.

use std::cell::RefCell;
use std::ffi::CString;
use std::future::Future;
use std::io::Read;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use android_activity::ndk::asset::AssetManager;
use cad_domain::{CadError, CadResult};
use cad_platform::fonts::{load_font_engine, FontLoadReport};
use cad_platform::{FontLoader, HostFuture};
use cad_ui_slint::CadView;

/// Logical base for fonts packaged under the APK's `assets/` directory.
pub const ASSET_BASE: &str = "asset://fonts/";

thread_local! {
    static ASSETS: RefCell<Option<AssetManager>> = const { RefCell::new(None) };
}

/// Record the activity asset manager once, before the UI starts.
pub fn set_asset_manager(manager: AssetManager) {
    ASSETS.with(|slot| *slot.borrow_mut() = Some(manager));
}

fn read_asset(url: &str) -> CadResult<Arc<[u8]>> {
    let path = url.strip_prefix(ASSET_BASE).unwrap_or(url);
    let name = CString::new(path)
        .map_err(|_| CadError::InvalidInput(format!("font asset path has NUL: {path}")))?;
    ASSETS.with(|slot| {
        let borrow = slot.borrow();
        let manager = borrow.as_ref().ok_or_else(|| {
            CadError::Unsupported(
                "Android font loading needs the activity AssetManager".to_string(),
            )
        })?;
        let mut asset = manager.open(&name).ok_or_else(|| {
            CadError::ResourceMissing(format!(
                "font asset not packaged (assets/{path}); fonts are not bundled (licence)"
            ))
        })?;
        let mut bytes = Vec::with_capacity(asset.length());
        asset.read_to_end(&mut bytes).map_err(|e| {
            CadError::ResourceMissing(format!("reading font asset assets/{path} failed: {e}"))
        })?;
        Ok(Arc::from(bytes.into_boxed_slice()))
    })
}

struct AssetFontLoader;

impl FontLoader for AssetFontLoader {
    fn load_font(&self, url: &str) -> HostFuture<'_, Arc<[u8]>> {
        let url = url.to_string();
        Box::pin(async move { read_asset(&url) })
    }
}

/// Asset reads resolve immediately, so a no-op-waker executor is enough;
/// this is not a general-purpose runtime.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// Load the referenced fonts into `view` from packaged assets.
pub fn install_fonts(view: Option<&CadView>, requested: &[String]) -> CadResult<FontLoadReport> {
    let (engine, report) = block_on(load_font_engine(&AssetFontLoader, requested, ASSET_BASE))?;
    if let Some(view) = view {
        if report.registered.is_empty() {
            view.clear_fonts();
        } else {
            view.set_fonts(engine);
        }
    }
    Ok(report)
}

/// Human-facing summary of one loading pass.
pub fn summary(report: &FontLoadReport, requested: usize) -> String {
    format!(
        "字体：目录 {}，引用 {}，计划 {}，注册 {}，失败 {}",
        report.catalog_entries,
        requested,
        report.planned.len(),
        report.registered.len(),
        report.failed.len(),
    )
}
