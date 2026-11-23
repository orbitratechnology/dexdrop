//! Android: reaching Java from Rust threads.
//!
//! DexDrop's Java bridges (`android/dev/dexdrop/*.java`, built into
//! `android/nsd.dex` by `scripts/build-nsd-dex.sh`) are embedded here and
//! loaded from memory (`InMemoryDexClassLoader`), with the process's Java VM
//! found through `JNI_GetCreatedJavaVMs` (public from Android 12) and the
//! application's `Context` by reflection inside the bridges.

use std::ffi::c_void;

use jni::{
    JNIEnv, JavaVM,
    objects::{JClass, JObject, JString, JValue},
    sys::{jint, jsize},
};

static DEX: &[u8] = include_bytes!("../android/nsd.dex");

/// The process's Java VM: Android 12+ exports `JNI_GetCreatedJavaVMs`
/// to apps from `libnativehelper.so`. `None` without one.
pub(crate) fn java_vm() -> Option<JavaVM> {
    type GetCreatedVms =
        unsafe extern "system" fn(*mut *mut jni::sys::JavaVM, jsize, *mut jsize) -> jint;
    // SAFETY: dlopen/dlsym with valid C strings; the symbol has this
    // signature (jni.h); the VM pointer it returns is valid for the
    // process's lifetime.
    unsafe {
        let lib = libc::dlopen(c"libnativehelper.so".as_ptr(), libc::RTLD_NOW);
        if lib.is_null() {
            return None;
        }
        let sym: *mut c_void = libc::dlsym(lib, c"JNI_GetCreatedJavaVMs".as_ptr());
        if sym.is_null() {
            return None;
        }
        let get: GetCreatedVms = std::mem::transmute(sym);
        let mut vm: *mut jni::sys::JavaVM = std::ptr::null_mut();
        let mut count: jsize = 0;
        if get(&mut vm, 1, &mut count) != jni::sys::JNI_OK || count < 1 || vm.is_null() {
            return None;
        }
        JavaVM::from_raw(vm).ok()
    }
}

/// Loads `class` (`dev.dexdrop.…`) from the embedded dex and creates one.
pub(crate) fn new_bridge<'a>(
    env: &mut JNIEnv<'a>,
    class: &str,
) -> jni::errors::Result<JObject<'a>> {
    let bytes = env.byte_array_from_slice(DEX)?;
    let buffer = env
        .call_static_method(
            "java/nio/ByteBuffer",
            "wrap",
            "([B)Ljava/nio/ByteBuffer;",
            &[JValue::Object(&bytes)],
        )?
        .l()?;
    let parent = env
        .call_static_method(
            "java/lang/ClassLoader",
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )?
        .l()?;
    let loader = env.new_object(
        "dalvik/system/InMemoryDexClassLoader",
        "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
        &[JValue::Object(&buffer), JValue::Object(&parent)],
    )?;
    let name = env.new_string(class)?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    let class = JClass::from(class);
    env.new_object(&class, "()V", &[])
}

/// The text of a pending Java exception (cleared), or of `err`.
pub(crate) fn describe(env: &mut JNIEnv<'_>, err: jni::errors::Error) -> String {
    if !env.exception_check().unwrap_or(false) {
        return err.to_string();
    }
    let Ok(thrown) = env.exception_occurred() else {
        return err.to_string();
    };
    let _ = env.exception_clear();
    env.call_method(&thrown, "toString", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
        .and_then(|s| env.get_string(&JString::from(s)).map(String::from))
        .unwrap_or_else(|_| err.to_string())
}

/// Gives `ndk-context` the process's Java VM and application `Context`,
/// once: Iroh's DNS resolver and network watcher read them, and panic in a
/// background task without them (NSH 0.6.1 does not set them; its next
/// release does, and then this must go). `false` without a Java VM (the
/// CLI run from a shell): internet transfers then stay off.
pub(crate) fn ensure_ndk_context() -> bool {
    use std::sync::OnceLock;
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        let Some(vm) = java_vm() else {
            return false;
        };
        let Ok(mut env) = vm.attach_current_thread() else {
            return false;
        };
        let context = env
            .call_static_method(
                "android/app/ActivityThread",
                "currentApplication",
                "()Landroid/app/Application;",
                &[],
            )
            .and_then(|v| v.l());
        let context = match context {
            Ok(context) if !context.is_null() => context,
            _ => {
                let _ = env.exception_clear();
                return false;
            }
        };
        let Ok(global) = env.new_global_ref(context) else {
            return false;
        };
        // SAFETY: the VM pointer is valid for the process's lifetime, and
        // the global reference is leaked so the context stays valid too.
        unsafe {
            ndk_context::initialize_android_context(
                vm.get_java_vm_pointer().cast::<c_void>(),
                global.as_obj().as_raw().cast::<c_void>(),
            );
        }
        std::mem::forget(global);
        true
    })
}
