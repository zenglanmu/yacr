//! Storage Access Framework (SAF) bridge — **Android only**.
//!
//! The Kotlin lane (`io.github.zenglanmu.yacr.MainActivity`) owns the `ACTION_OPEN_DOCUMENT`
//! / `ACTION_CREATE_DOCUMENT` pickers and the `ContentResolver` transfer, and
//! exposes the interaction through five **instance** methods:
//!
//! ```text
//! documentLaunchOpen(String?): void
//! documentLaunchSave(String): void
//! documentReadUri(String): void   // starts a worker read; poll the snapshot
//! documentWriteUri(String, byte[]): void
//! documentSnapshot(): Object[]
//! ```
//!
//! `documentSnapshot()` returns
//! `[Integer state, String? uri, String? name, String? error, byte[]? bytes]`
//! with state codes `NONE = 0`, `LOADING = 1`, `OK = 2`, `CANCELLED = 3`,
//! `FAILED = 4`. It **consumes** the terminal states (`OK`/`CANCELLED`/`FAILED`):
//! the next call answers `NONE` unless a new interaction is in flight. `LOADING`
//! is not consumed.
//!
//! `documentReadUri` only *starts* a read (the Kotlin worker thread owns the
//! `ContentResolver` call); the bytes arrive later as an `OK` snapshot. For an
//! `OK` result the Kotlin side supplies the document URI, a display name, and —
//! **for an open** — the already-read `bytes`. A save target has no bytes
//! (`null`). That nullness is the only open/save discriminator available to this
//! side, so it is part of the contract: `bytes != null` means an opened
//! document, `bytes == null` a save target. The Rust caller additionally arms the
//! expected [`cad_platform::FilePurpose`] and cross-wires nothing (see
//! `platform::saf_event`).
//!
//! # Threading
//!
//! The Activity object arrives as the implicit `this` of the frozen JNI
//! callbacks (`nativeSurfaceCreated`/`nativeAssetsReady`); there is no separate
//! Java helper class and no `FindClass`. [`capture`] stores the `JavaVM` and the
//! Activity's global reference once. Every later call attaches the current thread
//! when needed, pushes one bounded local frame, clears a pending Java exception on
//! error, and pops the frame. All failures become explicit `Err(String)` or
//! [`SafState::Failed`] — never a fabricated success.

use std::sync::OnceLock;

use jni::objects::{GlobalRef, JByteArray, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::platform::SafState;

/// The Java class that carries the SAF methods (also the manifest's Activity).
const ACTIVITY_CLASS: &str = "io.github.zenglanmu.yacr.MainActivity";

/// Stable logcat prefix of every SAF failure/warning from this module.
const LOG_PREFIX: &str = "yacr: saf";

// Snapshot state codes, mirrored from the Kotlin lane (`documentSnapshot`).
const STATE_NONE: i32 = 0;
const STATE_LOADING: i32 = 1;
const STATE_OK: i32 = 2;
const STATE_CANCELLED: i32 = 3;
const STATE_FAILED: i32 = 4;

/// The process-wide `JavaVM` captured from the first JNI callback.
static VM: OnceLock<JavaVM> = OnceLock::new();
/// The `MainActivity` global reference captured from the first JNI callback.
static ACTIVITY: OnceLock<GlobalRef> = OnceLock::new();

/// Record the `JavaVM` and the Activity from one of the frozen JNI callbacks.
///
/// Idempotent and panic-free: the first successful capture logs, every later call
/// is a no-op, and a failed capture is an explicit `logcat` error (the SAF calls
/// then refuse with "not captured" instead of pretending the picker opened).
pub(crate) fn capture(env: &mut JNIEnv, this: &JObject) {
    if VM.get().is_none() {
        match env.get_java_vm() {
            Ok(vm) => {
                if VM.set(vm).is_ok() {
                    log::info!("{LOG_PREFIX}: JavaVM captured");
                }
            }
            Err(error) => log_failure("JavaVM capture", &error.to_string()),
        }
    }
    if ACTIVITY.get().is_none() {
        match env.new_global_ref(this) {
            Ok(activity) => {
                if ACTIVITY.set(activity).is_ok() {
                    log::info!("{LOG_PREFIX}: activity captured ({ACTIVITY_CLASS})");
                }
            }
            Err(error) => log_failure("activity capture", &error.to_string()),
        }
    }
}

/// Launch the SAF open-document picker (`ACTION_OPEN_DOCUMENT`).
///
/// `initial_uri` is the last successfully opened/reopened document URI; Kotlin
/// turns it into `EXTRA_INITIAL_URI` so the picker starts in that document's
/// folder. `None` (a first open) leaves the picker at the provider default.
pub(crate) fn launch_open(initial_uri: Option<&str>) -> Result<(), String> {
    let outcome = with_env(|env, activity| {
        match initial_uri {
            Some(uri) => {
                let uri_obj = env.new_string(uri)?;
                env.call_method(
                    activity,
                    "documentLaunchOpen",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(uri_obj.as_ref())],
                )?;
            }
            None => {
                let null = JObject::null();
                env.call_method(
                    activity,
                    "documentLaunchOpen",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&null)],
                )?;
            }
        }
        Ok(())
    });
    if let Err(error) = &outcome {
        log_failure("documentLaunchOpen", error);
    }
    outcome
}

/// Launch the SAF create-document picker with a suggested file name.
pub(crate) fn launch_save(default_name: &str) -> Result<(), String> {
    let outcome = with_env(|env, activity| {
        let name = env.new_string(default_name)?;
        env.call_method(
            activity,
            "documentLaunchSave",
            "(Ljava/lang/String;)V",
            &[JValue::Object(name.as_ref())],
        )?;
        Ok(())
    });
    if let Err(error) = &outcome {
        log_failure("documentLaunchSave", error);
    }
    outcome
}

/// Start an asynchronous read of a document URI.
///
/// Kotlin owns the worker read: the slot becomes `LOADING` immediately and the
/// bytes arrive on a later [`poll`] as [`SafState::Opened`]. A JNI failure is an
/// explicit `Err`; a provider failure arrives as [`SafState::Failed`] from the
/// snapshot (never an empty document).
pub(crate) fn read_uri_async(uri: &str) -> Result<(), String> {
    let outcome = with_env(|env, activity| {
        let uri_obj = env.new_string(uri)?;
        env.call_method(
            activity,
            "documentReadUri",
            "(Ljava/lang/String;)V",
            &[JValue::Object(uri_obj.as_ref())],
        )?;
        Ok(())
    });
    if let Err(error) = &outcome {
        log_failure("documentReadUri", error);
    }
    outcome
}

/// Write `bytes` to a document URI (truncating any existing content).
pub(crate) fn write_uri(uri: &str, bytes: &[u8]) -> Result<(), String> {
    let outcome = with_env(|env, activity| {
        let uri_obj = env.new_string(uri)?;
        let array = env.byte_array_from_slice(bytes)?;
        env.call_method(
            activity,
            "documentWriteUri",
            "(Ljava/lang/String;[B)V",
            &[
                JValue::Object(uri_obj.as_ref()),
                JValue::Object(array.as_ref()),
            ],
        )?;
        Ok(())
    });
    if let Err(error) = &outcome {
        log_failure("documentWriteUri", error);
    }
    outcome
}

/// Read the Kotlin snapshot and decode it into one [`SafState`].
///
/// Every JNI-level failure becomes [`SafState::Failed`] (never a pending that
/// would hang the core).
pub(crate) fn poll() -> SafState {
    match with_env(read_snapshot) {
        Ok(state) => state,
        Err(error) => {
            log_failure("documentSnapshot", &error);
            SafState::Failed(error)
        }
    }
}

/// Decode `documentSnapshot()` into a [`SafState`].
fn read_snapshot(env: &mut JNIEnv, activity: &JObject) -> jni::errors::Result<SafState> {
    let snapshot = env
        .call_method(activity, "documentSnapshot", "()[Ljava/lang/Object;", &[])?
        .l()?;
    if snapshot.is_null() {
        // No snapshot array at all: still loading (not a fabricated failure).
        return Ok(SafState::Pending);
    }
    let snapshot = JObjectArray::from(snapshot);
    match read_int(env, &snapshot, 0)? {
        STATE_NONE | STATE_LOADING => Ok(SafState::Pending),
        STATE_CANCELLED => Ok(SafState::Cancelled),
        STATE_FAILED => {
            let detail = read_string(env, &snapshot, 3)?
                .unwrap_or_else(|| "the document picker reported a failure".to_string());
            Ok(SafState::Failed(detail))
        }
        STATE_OK => {
            let Some(uri) = read_string(env, &snapshot, 1)? else {
                return Ok(SafState::Failed(
                    "the document picker returned no document URI".to_string(),
                ));
            };
            let name = read_string(env, &snapshot, 2)?.unwrap_or_else(|| uri.clone());
            match read_bytes(env, &snapshot, 4)? {
                Some(bytes) => Ok(SafState::Opened { uri, name, bytes }),
                None => Ok(SafState::SaveTarget { uri, name }),
            }
        }
        other => Ok(SafState::Failed(format!(
            "unknown document picker state {other}"
        ))),
    }
}

/// Read the boxed `Integer` at `index` (a `null` element is `NONE`).
fn read_int(env: &mut JNIEnv, array: &JObjectArray, index: i32) -> jni::errors::Result<i32> {
    let element = env.get_object_array_element(array, index)?;
    if element.is_null() {
        return Ok(STATE_NONE);
    }
    env.call_method(&element, "intValue", "()I", &[])?.i()
}

/// Read the `String` at `index` (`null` → `None`).
fn read_string(
    env: &mut JNIEnv,
    array: &JObjectArray,
    index: i32,
) -> jni::errors::Result<Option<String>> {
    let element = env.get_object_array_element(array, index)?;
    if element.is_null() {
        return Ok(None);
    }
    let text: String = env.get_string(&JString::from(element))?.into();
    Ok(Some(text))
}

/// Read the `byte[]` at `index` (`null` → `None`).
fn read_bytes(
    env: &mut JNIEnv,
    array: &JObjectArray,
    index: i32,
) -> jni::errors::Result<Option<Vec<u8>>> {
    let element = env.get_object_array_element(array, index)?;
    if element.is_null() {
        return Ok(None);
    }
    let bytes = env.convert_byte_array(JByteArray::from(element))?;
    Ok(Some(bytes))
}

/// Run `f` with the captured JNI environment and Activity.
///
/// Mirrors the blueprint: reuse the thread's environment when attached, otherwise
/// attach permanently; one bounded local frame; a pending Java exception is
/// cleared before the error leaves this module.
fn with_env<R, F>(f: F) -> Result<R, String>
where
    F: for<'local> FnOnce(&mut JNIEnv<'local>, &JObject<'local>) -> jni::errors::Result<R>,
{
    let Some(vm) = VM.get() else {
        return Err("the Android JavaVM was not captured (no frozen JNI callback has run)".into());
    };
    let Some(activity) = ACTIVITY.get() else {
        return Err(
            "the Android activity was not captured (no frozen JNI callback has run)".into(),
        );
    };
    match vm.get_env() {
        Ok(mut env) => run(&mut env, activity, f),
        Err(_) => {
            let mut env = vm
                .attach_current_thread_permanently()
                .map_err(|error| format!("attach_current_thread failed: {error}"))?;
            run(&mut env, activity, f)
        }
    }
}

fn run<R, F>(env: &mut JNIEnv<'_>, activity: &GlobalRef, f: F) -> Result<R, String>
where
    F: for<'local> FnOnce(&mut JNIEnv<'local>, &JObject<'local>) -> jni::errors::Result<R>,
{
    // A global reference is valid in every frame; the borrow is only used for
    // JNI calls inside this call.
    let activity = unsafe { JObject::from_raw(activity.as_raw()) };
    // One local frame for the whole interaction: every local reference the calls
    // create (method arguments, the snapshot array and its elements, the byte
    // array) is released by the pop. Without it a permanently attached thread
    // accumulates references up to ART's 512 limit.
    env.push_local_frame(32)
        .map_err(|error| format!("push_local_frame failed: {error}"))?;
    let outcome = f(env, &activity);
    // A pending Java exception poisons every later JNI call; clear it before the
    // error leaves this module.
    if outcome.is_err() {
        if let Ok(true) = env.exception_check() {
            let _ = env.exception_clear();
        }
    }
    // SAFETY: the closure returns an owned Rust value; no local reference created
    // since the push is used after this point.
    unsafe {
        let _ = env.pop_local_frame(&JObject::null());
    }
    outcome.map_err(|error| error.to_string())
}

/// Log one SAF failure with the stable prefix.
fn log_failure(operation: &str, error: &str) {
    log::error!("{LOG_PREFIX}: {operation} failed: {error}");
}
