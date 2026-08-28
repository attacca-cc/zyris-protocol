# zyris-screen

The reference implementation of the `screen_capture` capability declared by
[`zyris-caps`](https://crates.io/crates/zyris-caps), for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol.

One type, `HostScreenCapture`. It enumerates the monitors of the machine the node runs on and
returns a still image of one of them, through `xcap` — XRandR on X11, Quartz on macOS, GDI on
Windows — or, in a wlroots Wayland session, `wl_output` and `zwlr_screencopy` directly.
`HostDisplays` is the same enumeration without the capture, for a node that also serves `input`.

```toml
[dependencies]
zyris = { version = "0.2", features = ["caps"] }
zyris-screen = "0.2"
```

```rust
use zyris::{Node, NodeKind};
use zyris_caps::{ImageFormat, ScreenCaptureServer};
use zyris_screen::HostScreenCapture;

let screen = HostScreenCapture::default()
    .with_format(ImageFormat::Jpeg)
    .with_max_width(1280);

let node = Node::builder()
    .name("workstation")
    .kind(NodeKind::Service)
    .capability(ScreenCaptureServer(screen))
    .build()?;
```

A full-resolution capture of a 4K display is several megabytes and a screenshot travels inline in
the response, so the default budget is `zyris::proto::INLINE_BLOB_MAX` and an oversized capture is
re-encoded smaller until it fits. When that happens the image says so: its description names the
display's real size and the factor to multiply image coordinates by, which is what a caller needs
before feeding a position from a screenshot into `input.move_to`.

## Two ways in on Wayland

A Wayland compositor does not let a client read the screen, and `ScreenBackend::detect` picks
between the two ways around that at construction.

On a wlroots-based compositor — Hyprland, Sway, river, Wayfire — the Wayland backend talks
`wl_output` and `zwlr_screencopy`, the same pair `grim` uses. That is not the same as letting
`xcap` do it: `xcap` enumerates over XRandR even in a Wayland session and then captures in
Wayland's coordinate space, and the two agree only while Xwayland can express the compositor's
layout. Put a monitor above the origin and its `y` goes negative, which X11 cannot represent, so
Xwayland flattens the arrangement into a row and every captured rectangle lands on the wrong
output.

Everywhere else — GNOME, KDE, anything without `zwlr_screencopy` — the probe fails and `xcap`
takes over, reaching the screen through `org.freedesktop.portal.Screenshot`. That interface has to
actually be offered: a session whose portal backend does not register it cannot capture at all.

## Building on Linux

`xcap` links against the display stack, so a Linux build needs development packages a headless
build of the rest of Zyris does not:

```sh
# Debian/Ubuntu
apt-get install pkg-config libclang-dev libxcb1-dev libxrandr-dev \
    libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev
```

Windows and macOS need nothing beyond a Rust toolchain. That is also why this crate's docs.rs
build is pinned to a Windows target: the doc builder runs Linux and has none of the above.

## License

MIT or Apache-2.0, at your option.
