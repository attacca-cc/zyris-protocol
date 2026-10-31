# zyris-capkit

Reference implementations of the [Zyris](https://github.com/attacca-cc/zyris-protocol) standard
capability catalogue declared by [`zyris-caps`](https://crates.io/crates/zyris-caps), one feature
each.

| Type | Feature | Implements | Details |
|---|---|---|---|
| `LocalFileIo` | `file-io` *(default)* | `file_io` | [docs/fs.md](docs/fs.md) |
| `PtyTerminal` | `terminal` *(default)* | `terminal`, over a real pseudo-terminal | [docs/terminal.md](docs/terminal.md) |
| `HostScreenCapture`, `HostDisplays` | `screen` | `screen_capture` | [docs/screen.md](docs/screen.md) |
| `EnigoInput` | `input`, `input-wayland`, `input-libei` | `input` — keyboard and mouse | [docs/input.md](docs/input.md) |
| `transfer::*` | `transfer`, `transfer-send`, `transfer-listen` | `file_transfer`, `peer_transfer` | [docs/transfer.md](docs/transfer.md) |

The two defaults link against nothing beyond the Rust ecosystem. `screen` and `input` are off
because they link the display stack — X11, Wayland, PipeWire, D-Bus — and a headless node should not
have to build any of it. `desktop` turns both on.

The sending and listening halves of file transfer are separate features because that is where the
dependency weight is: a node that only ever *receives* files should not have to build the QUIC
stack that makes them.

## Paths are resolved, not jailed

`resolve_under` gives a capability a default root, not a sandbox — an absolute path still leaves it.
A deployment that needs a fence has to enforce one; see the guard in
[`zyris-code`](https://github.com/attacca-cc/zyris-code) for how the reference client does it.

## Not published yet

The `input` feature pins a git fork of `enigo`, and crates.io refuses any manifest that names a git
source — even behind an optional feature. Until that fork is on the registry, depend on this crate
by git:

```toml
zyris-capkit = { git = "https://github.com/attacca-cc/zyris-protocol", features = ["screen"] }
```

`zyris-fs`, `zyris-terminal`, `zyris-screen` and `zyris-transfer` on crates.io are the 0.2.0
releases from before these implementations were merged here. Moving off them:

| Was | Is |
|---|---|
| `zyris_fs::LocalFileIo` | `zyris_capkit::LocalFileIo` |
| `zyris_terminal::PtyTerminal` | `zyris_capkit::PtyTerminal` |
| `zyris_transfer::*` | `zyris_capkit::transfer::*` (features `transfer`, `transfer-send`, `transfer-listen`) |
| `zyris_screen::*` | `zyris_capkit::screen::*`, or the root re-exports |
| `zyris_input::*` | `zyris_capkit::input::*`, or the root re-exports (features `input-wayland`, `input-libei`) |

## License

MIT or Apache-2.0, at your option.
