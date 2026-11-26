//! DexDrop's networking: TLS transport, the transfer sessions, nearby
//! discovery and the [`Engine`] service the UI drives.
//!
//! The engine runs on its own Tokio runtime and thread. Nothing here depends
//! on the UI toolkit, so it is tested headless and shared by the NSH app and
//! the `dexdrop` command-line tool.

#[cfg(target_os = "android")]
pub mod android_service;
pub mod apps;
pub mod assets;
pub mod discovery;
pub mod engine;
mod incoming;
#[cfg(target_os = "android")]
mod jvm;
pub mod media;
mod outgoing;
pub mod pairing;
pub mod remote;
pub mod session;
pub mod tls;
pub mod wire;

pub use dexdrop_core::api::*;
pub use engine::{Engine, EngineConfig, EngineError, EngineHandle, spawn, summary};
#[cfg(target_os = "android")]
pub use jvm::use_android_context;

/// The engine's event stream.
pub type Events = tokio::sync::mpsc::UnboundedReceiver<dexdrop_core::api::Event>;
