# zyris-capkit

Reference implementations of the [Zyris](https://github.com/attacca-cc/zyris-protocol) standard
capability catalogue declared by [`zyris-caps`](https://crates.io/crates/zyris-caps).

| Type | Feature | Implements |
|---|---|---|
| `LocalFileIo` | `file-io` *(default)* | `file_io` |
| `PtyTerminal` | `terminal` *(default)* | `terminal`, over a real pseudo-terminal |
| `HostScreenCapture`, `HostDisplays` | `screen` | `screen_capture` |
| `EnigoInput` | `input` | `input` — keyboard and mouse |
| `transfer` | `transfer`, `transfer-send`, `transfer-listen` | `file_transfer`, `peer_transfer` |

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

**This crate is not published.** It is `publish = false`: these are reference implementations,
and what a node offers is the node's own decision, not something a protocol crate should ship
by default. It is also what lets the desktop features pin a git fork of `enigo` — nothing on
crates.io may depend on a git revision, so a published capkit could not have carried them.

Depend on it by path or by git if you want the implementations:

```toml
zyris-capkit = { git = "https://github.com/attacca-cc/zyris-protocol" }
```

## License

MIT or Apache-2.0, at your option.
