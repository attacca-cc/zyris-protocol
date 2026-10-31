//! The reference implementations of the standard Zyris capabilities, one feature each.
//!
//! [`zyris_caps`] declares the capabilities; this crate implements them for the machine a node
//! runs on. Each implementation is a module behind a feature, so a node pays only for what it
//! offers — `screen` links X11, Wayland, D-Bus and PipeWire, while `file-io` and `terminal` are
//! pure Rust.
//!
//! | Feature | Module | Implements |
//! |---|---|---|
//! | `file-io` (default) | [`fs`] | `file_io`: [`LocalFileIo`] over `tokio::fs` |
//! | `terminal` (default) | [`terminal`] | `terminal`: [`PtyTerminal`] over `portable-pty` |
//! | `screen` | [`screen`] | `screen_capture`: [`HostScreenCapture`] over `xcap` or `zwlr_screencopy` |
//! | `input`, `input-wayland`, `input-libei` | [`input`] | `input`: [`EnigoInput`] over an `enigo` fork |
//! | `transfer`, `transfer-send`, `transfer-listen` | [`transfer`] | `file_transfer` and `peer_transfer` |
//!
//! `desktop` is `screen` + `input`.

// Never gated: `path` is std-only and `resolve_under` is what every capability taking a path
// resolves it with.
pub use zyris_caps::{path, resolve_under};

#[cfg(any(feature = "input", feature = "screen"))]
pub use zyris_caps::Displays;

#[cfg(feature = "file-io")]
pub mod fs;
#[cfg(feature = "file-io")]
pub use fs::LocalFileIo;

#[cfg(feature = "terminal")]
pub mod terminal;
#[cfg(feature = "terminal")]
pub use terminal::PtyTerminal;

#[cfg(feature = "screen")]
pub mod screen;
#[cfg(feature = "screen")]
pub use screen::{image, xcap, HostDisplays, HostScreenCapture, ScreenBackend};

#[cfg(feature = "input")]
pub mod input;
#[cfg(feature = "input")]
pub use input::{enigo, settings_for_session, EnigoInput};

#[cfg(feature = "transfer")]
pub mod transfer;
