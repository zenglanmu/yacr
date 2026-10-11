//! Storage Access Framework (SAF) bridge — Android only.
//!
//! The declared launcher activity is a `NativeActivity` subclass
//! (`dev.yacr.app.YacrNativeActivity`, see `android/dev/yacr/app/`) that mediates
//! the SAF `ACTION_OPEN_DOCUMENT` / `ACTION_CREATE_DOCUMENT` pickers and exposes
//! the outcome through static slots read back atomically via `snapshot()`. This
//! module is the Rust half: it drives the picker and reads/writes document URIs
//! through the `ContentResolver`.
//!
//! The interaction *purpose* (open vs save) is owned by the Rust side (the
//! caller's armed `FilePurpose`) and is not mirrored in Java, so the two sides
//! cannot disagree about it.
//!
//! All JNI work runs on the `android_main` (Slint event-loop) thread; the byte
//! transfer is done in Java (`readUri`/`writeUri`) and marshalled across as a
//! `byte[]`, which keeps the JNI surface small. Errors are returned as raw
//! (untranslated) strings; the caller localizes them with the shared catalog.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use cad_platform::FilePurpose;
use jni::objects::{GlobalRef, JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::{JNIEnv, JavaVM};

use crate::platform::SafResult;

/// The Java class that mediates SAF (also the manifest's declared activity).
const ACTIVITY_CLASS: &str = "dev.yacr.app.YacrNativeActivity";

// Result state codes, mirrored from the Java shim (`STATE_*` in
// `YacrNativeActivity.java`). The purpose pair is deliberately *not* mirrored:
// it is owned by the Rust caller.
const RESULT_NONE: i32 = 0;
const RESULT_OK: i32 = 1;
const RESULT_CANCELLED: i32 = 2;
const RESULT_FAILED: i32 = 3;

thread_local! {
    /// The `JavaVM*` captured from the `AndroidApp` (stable for the process).
    static VM: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// The activity's JNI global reference captured from the `AndroidApp`
    /// (borrowed, never deleted: android-activity owns it). Note that
    /// `ndk_context::android_context().context()` is the **Application**, not the
    /// Activity, so it cannot be used to call `startActivityForResult`.
    static ACTIVITY_INSTANCE: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// The SAF activity class as a JVM global reference, resolved once through the
    /// activity's own class loader. `FindClass` cannot see app classes from a
    /// native thread with no Java frame on the stack, so the class-loader route is
    /// the only reliable one.
    static ACTIVITY: RefCell<Option<GlobalRef>> = const { RefCell::new(None) };
}

/// Record the `JavaVM` and activity pointers from the `AndroidApp` handle.
///
/// Called once from `android_main` before `slint::android::init` consumes the
/// handle; the pointers stay valid for the process (android-activity and Slint
/// keep the handle alive).
pub(crate) fn set_context(vm: *mut c_void, activity: *mut c_void) {
    VM.with(|cell| cell.set(vm));
    ACTIVITY_INSTANCE.with(|cell| cell.set(activity));
}

/// Launch the SAF picker for an open (`save = false`) or save (`save = true`).
pub(crate) fn launch(save: bool, default_name: Option<&str>) -> Result<(), String> {
    with_env(|env, activity, class| {
        env.call_static_method(class, "begin", "()V", &[])?;
        let name_obj = match default_name {
            Some(name) => env.new_string(name)?,
            None => JString::from(JObject::null()),
        };
        env.call_static_method(
            class,
            "launch",
            "(Landroid/content/Context;ZLjava/lang/String;)V",
            &[
                JValue::Object(activity),
                JValue::Bool(u8::from(save)),
                JValue::Object(name_obj.as_ref()),
            ],
        )?;
        Ok(())
    })
}

/// Read the result snapshot and produce the outcome for the armed `purpose`.
pub(crate) fn poll(purpose: FilePurpose) -> SafResult {
    let outcome = with_env(|env, activity, class| {
        let snapshot = env
            .call_static_method(class, "snapshot", "()[Ljava/lang/Object;", &[])?
            .l()?;
        let snapshot = JObjectArray::from(snapshot);
        let result = read_int(env, &snapshot, 0)?;
        match result {
            RESULT_NONE => Ok(SafResult::Pending),
            RESULT_CANCELLED => Ok(SafResult::Cancelled),
            RESULT_FAILED => {
                let detail = read_string(env, &snapshot, 3)?
                    .unwrap_or_else(|| "the picker reported a failure".to_string());
                Ok(SafResult::Failed(detail))
            }
            RESULT_OK => {
                let Some(uri) = read_string(env, &snapshot, 1)? else {
                    return Ok(SafResult::Failed(
                        "the picker returned no document URI".into(),
                    ));
                };
                let name = read_string(env, &snapshot, 2)?.unwrap_or_else(|| uri.clone());
                match purpose {
                    FilePurpose::Save(_) => Ok(SafResult::SaveTarget { uri, name }),
                    FilePurpose::Open => {
                        let bytes = read_uri_jni(env, activity, class, &uri)?;
                        Ok(SafResult::Opened { uri, name, bytes })
                    }
                }
            }
            other => Ok(SafResult::Failed(format!(
                "unknown picker result code {other}"
            ))),
        }
    });
    outcome.unwrap_or_else(SafResult::Failed)
}

/// Write `bytes` to a document URI (truncating any existing content).
pub(crate) fn write_uri(uri: &str, bytes: &[u8]) -> Result<(), String> {
    with_env(|env, activity, class| {
        let uri_obj = env.new_string(uri)?;
        let array = env.byte_array_from_slice(bytes)?;
        env.call_static_method(
            class,
            "writeUri",
            "(Landroid/content/Context;Ljava/lang/String;[B)V",
            &[
                JValue::Object(activity),
                JValue::Object(uri_obj.as_ref()),
                JValue::Object(array.as_ref()),
            ],
        )?;
        Ok(())
    })
}

/// Read the whole document at a URI (used to re-open a known content URI).
pub(crate) fn read_uri(uri: &str) -> Result<Vec<u8>, String> {
    with_env(|env, activity, class| read_uri_jni(env, activity, class, uri))
}

fn read_uri_jni(
    env: &mut JNIEnv,
    activity: &JObject,
    class: &GlobalRef,
    uri: &str,
) -> jni::errors::Result<Vec<u8>> {
    let uri_obj = env.new_string(uri)?;
    let array = env
        .call_static_method(
            class,
            "readUri",
            "(Landroid/content/Context;Ljava/lang/String;)[B",
            &[JValue::Object(activity), JValue::Object(uri_obj.as_ref())],
        )?
        .l()?;
    env.convert_byte_array(JByteArray::from(array))
}

/// Read the `int` at `index` of the snapshot array (a boxed `Integer`).
fn read_int(env: &mut JNIEnv, array: &JObjectArray, index: i32) -> jni::errors::Result<i32> {
    let element = env.get_object_array_element(array, index)?;
    if element.is_null() {
        return Ok(RESULT_NONE);
    }
    env.call_method(&element, "intValue", "()I", &[])?.i()
}

/// Read the `String` at `index` of the snapshot array (`null` → `None`).
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

/// Run `f` with the JNI environment, the activity and the cached SAF class.
fn with_env<R>(
    f: impl FnOnce(&mut JNIEnv, &JObject, &GlobalRef) -> jni::errors::Result<R>,
) -> Result<R, String> {
    let vm_ptr = VM.with(Cell::get);
    if vm_ptr.is_null() {
        return Err("the Android JavaVM was not captured (android_main has not started)".into());
    }
    let activity_ptr = ACTIVITY_INSTANCE.with(Cell::get);
    if activity_ptr.is_null() {
        return Err("the Android activity was not captured (android_main has not started)".into());
    }
    let vm = unsafe { JavaVM::from_raw(vm_ptr.cast()) }
        .map_err(|error| format!("JavaVM::from_raw failed: {error}"))?;
    let activity_raw = activity_ptr as jni::sys::jobject;
    match vm.get_env() {
        Ok(mut env) => run(&mut env, activity_raw, f),
        Err(_) => {
            let mut guard = vm
                .attach_current_thread_permanently()
                .map_err(|error| format!("attach_current_thread failed: {error}"))?;
            run(&mut guard, activity_raw, f)
        }
    }
}

fn run<R>(
    env: &mut JNIEnv,
    activity_raw: jni::sys::jobject,
    f: impl FnOnce(&mut JNIEnv, &JObject, &GlobalRef) -> jni::errors::Result<R>,
) -> Result<R, String> {
    let activity = unsafe { JObject::from_raw(activity_raw) };
    // One local-reference frame for the whole interaction: every local ref the
    // calls below create (class lookups, method arguments, the snapshot array and
    // its elements) is released by the pop. Without this a permanently attached
    // thread accumulates refs across interactions up to ART's 512 limit.
    env.push_local_frame(32)
        .map_err(|error| error.to_string())?;
    let outcome = ensure_class(env, &activity).and_then(|()| {
        ACTIVITY.with(|slot| {
            let borrow = slot.borrow();
            let class = borrow.as_ref().expect("activity class cached above");
            f(env, &activity, class)
        })
    });
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

fn ensure_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
) -> jni::errors::Result<()> {
    if ACTIVITY.with(|slot| slot.borrow().is_some()) {
        return Ok(());
    }
    let class = load_class(env, activity)?;
    let global = env.new_global_ref(class)?;
    ACTIVITY.with(|slot| *slot.borrow_mut() = Some(global));
    Ok(())
}

fn load_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
) -> jni::errors::Result<JClass<'local>> {
    let activity_class = env.get_object_class(activity)?;
    let loader = env
        .call_method(
            &activity_class,
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )?
        .l()?;
    let name = env.new_string(ACTIVITY_CLASS)?;
    let loaded = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(name.as_ref())],
        )?
        .l()?;
    Ok(JClass::from(loaded))
}
