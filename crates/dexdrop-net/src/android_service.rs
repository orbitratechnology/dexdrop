//! Android: the foreground service that keeps DexDrop running while files
//! move (PRD 8.5, `android/dev/dexdrop/TransferService.java`), with an
//! ongoing notification of the transfers' progress. Android stops a
//! backgrounded app's network within seconds without one.
//!
//! The app calls [`show`] as transfers move and [`hide`] once none does.
//! Android only starts a foreground service while the app is in front
//! (Android 12+): a transfer that starts in the background goes on without
//! it. dexdrop-net's `Cargo.toml` declares the service and its permissions
//! (`[package.metadata.nsh.android]`).

use jni::objects::JValue;

use crate::jvm::{app_class, application, describe, java_vm};

const CLASS: &str = "dev.dexdrop.TransferService";

/// What the notification says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub title: String,
    pub text: String,
    /// Whole percent done; `None` while not known.
    pub percent: Option<u8>,
    /// Something is being sent (the status bar's arrow points up).
    pub upload: bool,
}

/// Starts the service, or updates its notification, with `status`. An
/// error says why it could not start (the app is in the background).
pub fn show(status: &Status) -> Result<(), String> {
    let vm = java_vm().ok_or("no Java VM in this process")?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    let result = env.with_local_frame(8, |env| -> jni::errors::Result<String> {
        let class = app_class(env, CLASS)?;
        let title = env.new_string(&status.title)?;
        let text = env.new_string(&status.text)?;
        let percent = status.percent.map_or(-1, i32::from);
        let answer = env
            .call_static_method(
                &class,
                "show",
                "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;IZ)Ljava/lang/String;",
                &[
                    JValue::Object(&application()),
                    JValue::Object(&title),
                    JValue::Object(&text),
                    JValue::Int(percent),
                    JValue::Bool(status.upload.into()),
                ],
            )?
            .l()?;
        Ok(env.get_string(&answer.into())?.into())
    });
    match result {
        Ok(answer) if answer == "ok" => Ok(()),
        Ok(why) => Err(why),
        Err(err) => Err(describe(&mut env, err)),
    }
}

/// Stops the service and takes its notification away.
pub fn hide() {
    let Some(vm) = java_vm() else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    let result = env.with_local_frame(4, |env| -> jni::errors::Result<()> {
        let class = app_class(env, CLASS)?;
        env.call_static_method(
            &class,
            "hide",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&application())],
        )?;
        Ok(())
    });
    if let Err(err) = result {
        tracing::warn!("transfer service: {}", describe(&mut env, err));
    }
}
