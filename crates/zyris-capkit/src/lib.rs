//! The reference implementations, as they used to be reached.
//!
//! **This crate is a facade with a deadline.** Every implementation it used to hold now lives in a
//! crate of its own — `zyris-fs`, `zyris-terminal`, `zyris-transfer`, `zyris-screen`,
//! `zyris-input` — and what is left here is re-exports at the old paths behind the old feature
//! names, so the three repositories that depend on this one can move at their own pace instead of
//! all in the same commit. When the last of them has moved, this crate goes.
//!
//! # Why the split happened
//!
//! One git dependency. `zyris-input` pins a fork of `enigo`, and crates.io refuses any manifest
//! whose graph names a git source — so while `LocalFileIo` and `PtyTerminal` shared a crate with
//! it, two implementations that are pure Rust and depend on nothing but `tokio` could not be
//! published either. Apart, only the input crate is held back.
//!
//! The weight came apart with it. `screen` links X11, Wayland, D-Bus and PipeWire and needs
//! development packages to build at all; `terminal` and `file-io` need nothing. A node that wants
//! a PTY now adds `zyris-terminal` and pays for a PTY, the way `ratatui-crossterm` is added
//! beside `ratatui`.
//!
//! # Moving off this crate
//!
//! | Was | Is |
//! |---|---|
//! | `zyris_capkit::LocalFileIo` | `zyris_fs::LocalFileIo` |
//! | `zyris_capkit::PtyTerminal` | `zyris_terminal::PtyTerminal` |
//! | `zyris_capkit::transfer::*` | `zyris_transfer::*` |
//! | `zyris_capkit::{HostScreenCapture, HostDisplays, ScreenBackend}` | `zyris_screen::*` |
//! | `zyris_capkit::{EnigoInput, settings_for_session}` | `zyris_input::*` |
//! | `zyris_capkit::resolve_under` | `zyris_caps::resolve_under` |
//! | `zyris_capkit::Displays` | `zyris_caps::Displays` |
//!
//! The last two moved *down* rather than sideways. They are not behaviour: they are what two
//! independent implementations of one declaration have to agree on — what a caller's path means,
//! and what a display layout is — so they belong with the declarations, and a consumer that
//! already depends on `zyris-caps` needs no new dependency line to keep them.
//!
//! Feature names carried over unchanged here, but they are shorter in the crates they now name:
//! `transfer-send` is `zyris-transfer/send`, `input-libei` is `zyris-input/libei`. The crate name
//! already says which subject it is about.

// Never gated, because it never was: `path` is std-only and `resolve_under` is what every
// capability taking a path resolves it with.
pub use zyris_caps::{path, resolve_under};

#[cfg(any(feature = "input", feature = "screen"))]
pub use zyris_caps::Displays;

#[cfg(feature = "file-io")]
pub use zyris_fs::LocalFileIo;

#[cfg(feature = "terminal")]
pub use zyris_terminal::PtyTerminal;

#[cfg(feature = "screen")]
pub use zyris_screen::{image, xcap, HostDisplays, HostScreenCapture, ScreenBackend};

// A module rather than a name list: consumers reach into `transfer::listen::serve_peers` and
// `transfer::peer::TransferConfig`, and re-exporting the crate keeps every one of those paths.
#[cfg(feature = "transfer")]
pub use zyris_transfer as transfer;

#[cfg(feature = "input")]
pub use zyris_input::{enigo, settings_for_session, EnigoInput};
