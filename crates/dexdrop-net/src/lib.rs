//! DexDrop's networking: TLS transport, the transfer sessions, nearby
//! discovery and the [`Engine`] service the UI drives.
//!
//! The engine runs on its own Tokio runtime and thread. Nothing here depends
//! on the UI toolkit, so it is tested headless and shared by the NSH app and
//! the `dexdrop` command-line tool.

pub mod discovery;
pub mod engine;
mod incoming;
mod outgoing;
pub mod pairing;
pub mod session;
pub mod tls;
pub mod wire;

pub use dexdrop_core::api::*;
pub use engine::{Engine, EngineConfig, EngineError, EngineHandle, spawn, summary};

/// The engine's event stream.
pub type Events = tokio::sync::mpsc::UnboundedReceiver<dexdrop_core::api::Event>;
