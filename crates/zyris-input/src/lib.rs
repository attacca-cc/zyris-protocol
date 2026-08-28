//! The reference implementation of the Zyris `input` capability.
//!
//! [`zyris_caps::Input`] is the contract — type text, press a chord, move the pointer, click,
//! scroll. This crate is one answer to it: [`EnigoInput`] drives the real keyboard and pointer of
//! the machine the node runs on, through [`enigo`] — XTEST on X11, `wlr-virtual-*` on Wayland,
//! `SendInput` on Windows, `CGEvent` on macOS.
//!
//! ```no_run
//! use zyris::{Node, NodeKind};
//! use zyris_caps::{Display, InputServer};
//! use zyris_input::EnigoInput;
//!
//! // Where the monitors are. `zyris-screen`'s `HostDisplays` reports this for the running host;
//! // a node that does not serve `screen_capture` can hand over any layout it knows.
//! let displays = vec![Display {
//!     id: "DP-1".into(),
//!     name: "DP-1".into(),
//!     x: 0,
//!     y: 0,
//!     width: 1920,
//!     height: 1080,
//!     scale_factor: 1.0,
//!     primary: true,
//! }];
//!
//! let node = Node::builder()
//!     .name("workstation")
//!     .kind(NodeKind::Service)
//!     .capability(InputServer(EnigoInput::new(displays).unwrap()))
//!     .build()
//!     .unwrap();
//! ```
//!
//! `move_to` is display-local — the position is in the pixels a `screen_capture.screenshot` of that
//! display is made of, so a point read off a screenshot goes straight in. That is why construction
//! takes a [`Displays`](zyris_caps::Displays) rather than enumerating monitors here: the
//! enumerators link the whole display stack, and a node that only offers `input` should not have to
//! build any of it.
//!
//! # Two extra backends on Linux, both off by default
//!
//! | Feature | Backend | The session it is for |
//! |---|---|---|
//! | *(none)* | XTEST, via `x11rb` | X11, and Xwayland where the compositor honours it |
//! | `wayland` | `wlr-virtual-keyboard` / `-pointer` | wlroots — Hyprland, Sway, river, Wayfire |
//! | `libei` | libei through the RemoteDesktop portal | GNOME and KDE, which drop XTEST silently |
//!
//! `wayland` is also what makes a second monitor reachable in a wlroots session:
//! `wlr-virtual-pointer` positions the cursor *within* the output its pointer was created against,
//! so there is no whole-desktop coordinate to aim at and the output has to be addressed by name.
//!
//! # Building on Linux
//!
//! `enigo` depends on the `xkbcommon` crate unconditionally for every `unix` target except macOS,
//! and that crate is a bare `#[link(name = "xkbcommon")]` with no build script and no `pkg-config`
//! probe — so it neither finds the library for you nor tells you what is missing beyond a linker
//! error. `libxkbcommon` and its development symlink have to be present:
//!
//! ```text
//! # Debian/Ubuntu
//! apt-get install libxkbcommon-dev
//! ```
//!
//! `wayland` adds `libwayland-client` (`libwayland-dev`) on top of that; `libei` is pure Rust
//! and needs nothing further at link time, though it does need a session offering the
//! `org.freedesktop.portal.RemoteDesktop` portal to do anything at run time. Windows and macOS
//! need nothing beyond a Rust toolchain.
//!
//! # This crate is not published
//!
//! It is `publish = false`, and unlike `zyris-capkit` — where that was a decision about what a
//! protocol repository should ship — here it is a constraint. `enigo` is depended on as a git fork
//! carrying two patches that are not upstream, and crates.io refuses any crate whose manifest names
//! a git source. Getting those patches upstream is the only way out; see the comment on the
//! dependency in `Cargo.toml`.

mod chord;
mod display;
mod input;

/// Wholesale, and deliberately: `EnigoInput::with_settings` takes an [`enigo::Settings`] and
/// `restore_token` speaks in the portal's terms, so a caller has to be able to name the crate
/// those came from. Pinning `enigo` a second time downstream is how two incompatible copies end
/// up linked in — and with a git fork in the picture, the second copy would be the wrong one.
pub use enigo;
pub use input::{settings_for_session, EnigoInput};
