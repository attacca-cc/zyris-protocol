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

## Coordinates: one space, and it is the picture's

Every number in a `Display`, every `Region`, and both coordinates of `input.move_to` are in one
space — **captured pixels**, the pixels a `screenshot` of that display is actually made of.
`Display::width` and `height` are by definition the size of the image an uncropped, undownscaled
capture returns. `x` and `y` place that display's top-left corner on the virtual desktop in the
same space, so adding them to a display-local position gives a whole-desktop one and subtracting
them reverses it. `scale_factor` is reported so a caller can talk about the display the way its
desktop environment does; it is never a factor to apply to the fields above.

Where a platform's advertised geometry and its captured image disagree, the image is the contract
and the geometry is the defect. Two places that bites:

- **GNOME's Xwayland at a fractional scale.** `xcap` divides the RandR geometry by a scale it takes
  from `wl_output`, but under Xwayland mutter inflates that geometry by an *integer* scale
  (`ceil` of the highest monitor scale) — so numerator and denominator come from different
  windowing systems and no factor relates the result to the picture. Measured on a 1920x1080 panel
  at 125%: `xcap` says 2457x1382 at scale 1.25, that product is 3071x1728, and a capture is
  1920x1080. This crate reads `wl_output`'s current mode there instead of deriving. The error is
  `ceil(s)/s`, which is 1 at every integer scale — so 100% and 200% were always fine, and only
  fractional scaling was ever wrong.
- **macOS in a scaled Retina mode**, in the other direction: the system renders to an oversized
  backing store and hands *that* back, so the picture is larger than the physical panel. It is
  still the right answer, because it is what you receive and what a region is cropped from.

Two things are known wrong and are not fixed here. A **second monitor on GNOME** gets a correct
advertised size and still the wrong pixels — `xcap` crops the compositor's screenshot from the
origin rather than from the monitor's rectangle, which is upstream's to fix. **Rotation** is
unverified: `wl_output`'s mode is pre-transform where RandR's geometry is post-transform, so a
rotated output may now report a transposed size.

The pointer half has a gap of its own, in the other crate: `zyris-input`'s `libei` backend — the
only one that works on GNOME and KDE — passes a position to libei without resolving it against the
device region, and that region is in logical pixels. So on those two desktops a fractionally scaled
`move_to` lands short even though the display is now described correctly. It is off by default and
documented in `zyris-input`'s README.

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
