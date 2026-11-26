//! Android: reaching Java from Rust threads.
//!
//! NSH sets `ndk-context` to the process's Java VM and the application's
//! `Context` before the app's Rust code runs (and panics if anyone sets it
//! again, so DexDrop never does). The app says so with
//! [`use_android_context`] before it starts the engine; a process without
//! it (the `dexdrop` CLI run from a shell) has no VM: discovery then uses
//! mdns-sd and internet transfers stay off.
//!
//! DexDrop's Java bridges (`android/dev/dexdrop/*.java`, built into
//! `android/nsd.dex` by `scripts/build-nsd-dex.sh`) are packaged with the
//! app as an Android library: this crate lists the dex under
//! `[package.metadata.nsh.android]` in its `Cargo.toml`, and `nsh package`
//! carries it. Their classes come from the application's class loader: a
//! thread Rust started finds only the system's classes through `FindClass`.

use std::sync::atomic::{AtomicBool, Ordering};

use jni::{
    JNIEnv, JavaVM,
    objects::{JClass, JObject, JString, JValue},
};

/// Set by [`use_android_context`].
static CONTEXT_SET: AtomicBool = AtomicBool::new(false);

/// Tells the engine that this process's `ndk-context` holds its Java VM and
/// application `Context`, as NSH sets them before an app's code runs. Call
/// it once from the app, before [`crate::Engine::start`]. Without it the
/// engine reaches no Java: discovery uses mdns-sd, internet transfers stay
/// off, and listing apps is unavailable.
pub fn use_android_context() {
    CONTEXT_SET.store(true, Ordering::Release);
    // The identity key goes to the Android Keystore from now on.
    dexdrop_core::keystore::use_android_context();
}

/// The process's Java VM, from `ndk-context`; `None` before
/// [`use_android_context`].
pub(crate) fn java_vm() -> Option<JavaVM> {
    if !CONTEXT_SET.load(Ordering::Acquire) {
        return None;
    }
    let context = ndk_context::android_context();
    // SAFETY: NSH set a valid VM pointer, which lives as long as the
    // process.
    unsafe { JavaVM::from_raw(context.vm().cast()) }.ok()
}

/// Whether `ndk-context` is set (Iroh's DNS resolver and network watcher
/// read it and panic in a background task without it).
pub(crate) fn has_context() -> bool {
    CONTEXT_SET.load(Ordering::Acquire)
}

/// The application's `Context`, which NSH keeps for the life of the process.
pub(crate) fn application<'a>() -> JObject<'a> {
    // SAFETY: a global reference NSH keeps for the life of the process; a
    // `JObject` made from it never deletes it.
    unsafe { JObject::from_raw(ndk_context::android_context().context().cast()) }
}

/// One of DexDrop's classes (`dev.dexdrop.…`), loaded by the application's
/// class loader from the dex the package carries.
pub(crate) fn app_class<'a>(env: &mut JNIEnv<'a>, class: &str) -> jni::errors::Result<JClass<'a>> {
    let loader = env
        .call_method(
            application(),
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )?
        .l()?;
    let name = env.new_string(class)?;
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    Ok(JClass::from(class))
}

/// Creates one bridge `class` (`dev.dexdrop.…`), given the application's
/// `Context` (the bridges find nothing by reflection).
pub(crate) fn new_bridge<'a>(
    env: &mut JNIEnv<'a>,
    class: &str,
) -> jni::errors::Result<JObject<'a>> {
    let class = app_class(env, class)?;
    env.new_object(
        &class,
        "(Landroid/content/Context;)V",
        &[JValue::Object(&application())],
    )
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

/// One of DexDrop's Java bridges (`dev.dexdrop.…`), made on first use and
/// kept: its methods take and return strings. Calls run concurrently (a
/// call that waits for the user does not hold up the others).
pub(crate) struct Bridge {
    class: &'static str,
    made: std::sync::OnceLock<(JavaVM, jni::objects::GlobalRef)>,
    making: std::sync::Mutex<()>,
}

impl Bridge {
    pub(crate) const fn new(class: &'static str) -> Self {
        Bridge {
            class,
            made: std::sync::OnceLock::new(),
            making: std::sync::Mutex::new(()),
        }
    }

    fn get(&self) -> Result<&(JavaVM, jni::objects::GlobalRef), String> {
        if let Some(made) = self.made.get() {
            return Ok(made);
        }
        let _one = self.making.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(made) = self.made.get() {
            return Ok(made);
        }
        let vm = java_vm().ok_or("Android's services are not reachable.")?;
        let bridge = {
            let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
            match new_bridge(&mut env, self.class).and_then(|b| env.new_global_ref(b)) {
                Ok(b) => b,
                Err(err) => return Err(describe(&mut env, err)),
            }
        };
        Ok(self.made.get_or_init(|| (vm, bridge)))
    }

    /// Calls `method(String…)` and returns its text (empty for `null`).
    pub(crate) fn call(&self, method: &str, args: &[&str]) -> Result<String, String> {
        let (vm, bridge) = self.get()?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let signature = format!(
            "({})Ljava/lang/String;",
            "Ljava/lang/String;".repeat(args.len())
        );
        let result = env.with_local_frame(16, |env| -> jni::errors::Result<String> {
            let strings = args
                .iter()
                .map(|a| env.new_string(a))
                .collect::<Result<Vec<_>, _>>()?;
            let values: Vec<JValue> = strings.iter().map(|s| JValue::Object(s)).collect();
            let out = env
                .call_method(bridge.as_obj(), method, &signature, &values)?
                .l()?;
            if out.is_null() {
                return Ok(String::new());
            }
            Ok(env.get_string(&JString::from(out))?.into())
        });
        result.map_err(|err| describe(&mut env, err))
    }
}
