//! Where the identity's private key is kept (PRD 9.9).
//!
//! The private key goes to the system's credential store where one works
//! without prompting the user, and otherwise stays in the app-private
//! identity file (mode 0600 on Unix, inside the app's sandbox on phones):
//!
//! | Platform | Key in | Why |
//! |---|---|---|
//! | Windows | Credential Manager (DPAPI, per user) | |
//! | Linux | Secret Service (GNOME Keyring, KWallet) when one answers | headless systems have none: file |
//! | iOS | Keychain, data-protection store (per app) | |
//! | macOS | file | the Keychain ties items to the code signature, so ad-hoc-signed builds would prompt after every update; Keychain with signed releases |
//! | Android | the Android Keystore: the key encrypted in the app's SharedPreferences under a Keystore key that never leaves it (`android-native-keyring-store`) | needs the Java VM and `Context` NSH puts in `ndk-context` ([`use_android_context`]); without them (the CLI from a shell) the file |
//!
//! `DEXDROP_KEY_STORE=file` keeps the key in the file everywhere (tests,
//! portable setups). A key already in the file moves to the system's store
//! the first time the store takes it and gives it back
//! (`Identity::load_or_create_with`); until then it stays in the file.
//! Threat model: docs/SECURITY.md.

/// Android: set once the process's `ndk-context` holds its Java VM and
/// application `Context` ([`use_android_context`]).
#[cfg(target_os = "android")]
static ANDROID_CONTEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Android: the process's `ndk-context` holds its Java VM and application
/// `Context` (NSH sets them before an app's code runs), so the Android
/// Keystore can be reached. Without it keys stay in the file: the store
/// would panic reading an unset context.
#[cfg(target_os = "android")]
pub fn use_android_context() {
    ANDROID_CONTEXT.store(true, std::sync::atomic::Ordering::Release);
}

#[cfg(target_os = "android")]
fn android_context() -> bool {
    ANDROID_CONTEXT.load(std::sync::atomic::Ordering::Acquire)
}

/// Where an identity's private key is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStorage {
    /// In the app-private identity file.
    File,
    /// In the system's credential store.
    System,
}

impl KeyStorage {
    /// This platform's choice (table above).
    pub fn platform_default() -> Self {
        if std::env::var("DEXDROP_KEY_STORE").as_deref() == Ok("file") {
            return Self::File;
        }
        #[cfg(target_os = "android")]
        if !android_context() {
            return Self::File;
        }
        if cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "ios",
            target_os = "android"
        )) {
            Self::System
        } else {
            Self::File
        }
    }

    /// For people: where the key is.
    pub fn describe(self) -> &'static str {
        match self {
            Self::File => "app-private file",
            Self::System => "system credential store",
        }
    }
}

/// The service name entries are stored under; the account is the peer ID.
#[cfg_attr(
    not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "ios",
        target_os = "android"
    )),
    allow(dead_code)
)]
const SERVICE: &str = "dev.dexdrop.identity";

#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "ios",
    target_os = "android"
))]
mod system {
    use std::sync::{Arc, OnceLock};

    use keyring_core::{CredentialStore, Entry, Error};

    fn store() -> Result<Arc<CredentialStore>, String> {
        // Checked before the store is made (and kept): reading an unset
        // ndk-context panics.
        #[cfg(target_os = "android")]
        if !super::android_context() {
            return Err("no Android context in this process".to_owned());
        }
        static STORE: OnceLock<Result<Arc<CredentialStore>, String>> = OnceLock::new();
        STORE
            .get_or_init(|| {
                #[cfg(target_os = "windows")]
                let store = windows_native_keyring_store::Store::new();
                #[cfg(target_os = "linux")]
                let store = zbus_secret_service_keyring_store::Store::new();
                #[cfg(target_os = "ios")]
                let store = apple_native_keyring_store::protected::Store::new();
                // Its own SharedPreferences file and Keystore alias
                // (`keyring-dexdrop`), never another library's.
                #[cfg(target_os = "android")]
                let store = android_native_keyring_store::Store::new_with_configuration(
                    &std::collections::HashMap::from([("name", "dexdrop")]),
                );
                store
                    .map(|s| s as Arc<CredentialStore>)
                    .map_err(|e| e.to_string())
            })
            .clone()
    }

    fn entry(account: &str) -> Result<Entry, String> {
        Entry::new_in_store(&store()?, super::SERVICE, account).map_err(|e| e.to_string())
    }

    pub fn put(account: &str, secret: &[u8]) -> Result<(), String> {
        entry(account)?
            .set_secret(secret)
            .map_err(|e| e.to_string())
    }

    pub fn get(account: &str) -> Result<Option<Vec<u8>>, String> {
        match entry(account)?.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "ios",
    target_os = "android"
)))]
mod system {
    const NONE: &str = "this platform has no system credential store DexDrop uses";

    pub fn put(_: &str, _: &[u8]) -> Result<(), String> {
        Err(NONE.into())
    }

    pub fn get(_: &str) -> Result<Option<Vec<u8>>, String> {
        Err(NONE.into())
    }

    pub fn delete(_: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Stores `secret` for `account` and reads it back: a store that accepts a
/// write but cannot return it (no Secret Service running) does not count.
pub(crate) fn put_checked(account: &str, secret: &[u8]) -> Result<(), String> {
    system::put(account, secret)?;
    match system::get(account)? {
        Some(read) if read == secret => Ok(()),
        _ => {
            let _ = system::delete(account);
            Err("the credential store did not return what was stored".into())
        }
    }
}

/// The secret stored for `account`; `None` when there is none.
pub(crate) fn get(account: &str) -> Result<Option<Vec<u8>>, String> {
    system::get(account)
}

/// Removes the secret stored for `account`, if any.
pub(crate) fn delete(account: &str) -> Result<(), String> {
    system::delete(account)
}
