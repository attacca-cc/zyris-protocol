# zyris-input

The reference implementation of the `input` capability declared by
[`zyris-caps`](https://crates.io/crates/zyris-caps), for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol.

One type, `EnigoInput`. It types text, presses a chord such as `ctrl+shift+t`, moves the pointer,
clicks and scrolls on the machine the node runs on, through
[`enigo`](https://crates.io/crates/enigo) — XTEST on X11, `wlr-virtual-*` on Wayland, `SendInput`
on Windows, `CGEvent` on macOS.

```rust
use zyris::{Node, NodeKind};
use zyris_caps::InputServer;
use zyris_input::EnigoInput;
use zyris_screen::HostDisplays;

let node = Node::builder()
    .name("workstation")
    .kind(NodeKind::Service)
    .capability(InputServer(EnigoInput::new(HostDisplays::default())?))
    .build()?;
```

`move_to` is display-local: the position is in the pixels a `screen_capture.screenshot` of that
display is made of, so a point read off a screenshot goes straight in, and one that falls outside
the display is rejected rather than landing on a neighbour. That is why construction takes a
`zyris_caps::Displays` instead of enumerating monitors itself — the enumerators link the whole
display stack, and a node offering only `input` should not have to build any of it. A node that
does not serve `screen_capture` can pass any `Vec<Display>` it knows.

`EnigoInput::new` fails on a headless host, which is the answer a node wants at startup: if there
is no display server, do not announce `input` at all.

## Coordinates

`move_to` takes display-local **captured** pixels — the pixels a `screen_capture.screenshot` of
that display is made of, and the same space `Display::width`, `Display::x` and `Region` are in. So
a position read off a screenshot goes in unchanged: subtract the region origin and multiply by the
factor the image's description names, and nothing else. `Display::scale_factor` is not applied on
the way, and the display's own position on the desktop is added here rather than by the caller.

That coupling is the reason `Displays` lives in `zyris-caps` rather than in either implementation,
and it is easy to break from the other side: a supplier that reports a display in some space other
than its picture's aims the pointer somewhere the screenshot never showed. `zyris-screen`'s README
has the two platforms where that has actually happened.

### Known wrong: `libei`, so GNOME and KDE

The `libei` backend does not honour that. libei hands each device a region, and its offsets and
extents are in **logical** pixels with the physical scale alongside — but `move_mouse` calls
`motion_absolute` with the caller's numbers untouched, and the regions it collected are never read
at all. So on a fractionally scaled GNOME or KDE session the pointer lands short of where the
screenshot said, by the display's scale factor.

Nothing above is wrong about the contract; this backend simply does not meet it yet, and it is the
only backend that works on those two desktops. It is off by default, so a build that has not opted
into `libei` is unaffected. Fixing it means resolving the position against the device's own region
before the call, in the fork — not in this crate, which cannot see the regions.

## Two extra backends on Linux, both off by default

| Feature | Backend | The session it is for |
|---|---|---|
| *(none)* | XTEST, via `x11rb` | X11, and Xwayland where the compositor honours it |
| `wayland` | `wlr-virtual-keyboard` / `-pointer` | wlroots — Hyprland, Sway, river, Wayfire |
| `libei` | libei through the RemoteDesktop portal | GNOME and KDE, which drop XTEST silently |

Neither is a nicety. GNOME and KDE ignore XTEST input arriving through Xwayland — silently, so
every call succeeds and nothing moves — and `libei` is the only path that works there.
`wayland` is what makes a second monitor reachable under wlroots: `wlr-virtual-pointer` positions
the cursor *within* the output its pointer was created against, so there is no whole-desktop
coordinate to aim at and the output has to be addressed by name.

`libei` remembers its grant. The first connection opens the portal's permission dialog; save
`EnigoInput::restore_token()` afterwards and pass it to `settings_for_session` next time, and the
dialog does not come back.

## Building on Linux

`enigo` depends on the `xkbcommon` crate unconditionally for every `unix` target except macOS, and
that crate is a bare `#[link(name = "xkbcommon")]` with no build script and no `pkg-config` probe —
so it neither finds the library for you nor says what is missing beyond a linker error.
`libxkbcommon` and its development symlink have to be there:

```sh
# Debian/Ubuntu
apt-get install libxkbcommon-dev
```

`wayland` adds `libwayland-client` (`libwayland-dev`) on top of that. `libei` is pure Rust and
needs nothing further at link time, though at run time it needs a session offering the
`org.freedesktop.portal.RemoteDesktop` portal. Windows and macOS need nothing beyond a Rust
toolchain — on macOS the process needs Accessibility permission instead, and the first call opens
the system prompt asking for it.

## This crate is not published

It is `publish = false`, and unlike the other implementation crates that is not a choice about what
belongs on crates.io. `enigo` is depended on as a git fork carrying two patches that are not
upstream — per-output absolute motion for `wlr-virtual-pointer`, and Shift for level-1 keysyms on
libei, without which `!` arrives as `1` under GNOME — and crates.io refuses any crate whose
manifest names a git source. Getting those two patches upstream is the only way out; a `[patch]`
section here or downstream is not, because it builds on exactly one machine.

Depend on it by git in the meantime:

```toml
zyris-input = { git = "https://github.com/attacca-cc/zyris-protocol" }
```

## License

MIT or Apache-2.0, at your option.
