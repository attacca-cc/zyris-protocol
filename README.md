# Zyris

An expandable computer protocol. A **Zyris node** is any machine that dials a server over one
websocket, announces typed **capabilities** — named, versioned sets of tools with JSON-Schema'd
arguments — and, on the same connection, consumes the capabilities the server announces back.

The protocol is direction-symmetric. There is no client role and no server role once the handshake
is done: either peer may call the other, open streams, and change what it offers mid-session.

[Attacca](https://attacca.cc) is the reference deployment: a node's tools become tools its owner's
agents can call, and the server announces `attacca_api` so the same node can drive agents and
sessions in return.

## One dependency

```toml
[dependencies]
zyris = { version = "0.2", features = ["full"] }
```

```rust
use zyris::{Node, NodeKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let link = Node::builder()
        .kind(NodeKind::Service)
        .capability(MyServer(my_impl))
        .build()?
        .connect(zyris::DEFAULT_SERVER_URL, std::env::var("ZYRIS_NODE_TOKEN")?)
        .await?;

    link.wait_closed().await?;
    Ok(())
}
```

## The crates

| Crate | Reached as | What it is |
|---|---|---|
| [`zyris`](crates/zyris) | — | The whole stack behind one name. The runtime at the root, everything above it as a module behind a feature. **Depend on this one.** |
| [`zyris-core`](crates/zyris-core) | `zyris::*` | The node runtime and client — connection state machine, transports, capability announce/accept, the dial/reconnect loop, and device-grant enrollment. |
| [`zyris-proto`](crates/zyris-proto) | `zyris::proto` | Wire types only: envelopes, frames, the handshake, datums and blobs. No I/O, no async. |
| [`zyris-macros`](crates/zyris-macros) | `zyris::capability` | The `#[zyris::capability]` proc-macro. Not a direct dependency. |
| [`zyris-caps`](crates/zyris-caps) | `zyris::caps` | The standard capability catalogue — `terminal`, `file_io`, `input`, `screen_capture`, `browser_chrome`, `file_transfer`. Declarations only: no tokio, no OS dependencies, cheap for a client to depend on. |
| [`zyris-capkit`](crates/zyris-capkit) | — | Reference implementations of that catalogue: `LocalFileIo` and `PtyTerminal` by default, plus `HostScreenCapture` and `EnigoInput` behind the `screen` and `input` features. **Not published to crates.io**: what a node offers is the node's decision, and this crate pins a git fork of `enigo`, which a published crate may not do. Depend on it by git and name it directly. |
| [`zyris-attacca`](crates/zyris-attacca) | `zyris::attacca` | The `attacca_api` capability: the one surface that runs the other way, announced by the server rather than by a node. |
| [`zyris-p2p`](crates/zyris-p2p) | `zyris::p2p` | Transport that carries Zyris over a direct node-to-node connection. |
| [`zyris-hello`](crates/zyris-hello) | — | A complete node in two short files. The thing to copy. Not published to crates.io on purpose: a crate to edit, not a binary to install. |

**The runtime ships as `zyris-core`, not as `zyris`.** Every crate above the runtime depends on the
runtime, so the runtime cannot depend on any of them — Cargo has no cycles. `zyris` re-exports
`zyris-core` whole, and `zyris::Node` and `zyris_core::Node` are one type, so a capability written
against either serves the other. Crates in that position — a catalogue, or an implementation of one
— depend on `zyris-core` directly.

## A capability

One trait. The macro turns it into the descriptor, a server wrapper, and a client:

```rust
#[zyris::capability(name = "hello", version = 1)]
pub trait Hello {
    /// Return a random friendly greeting, optionally addressed to `name`.
    async fn greet(&self, name: Option<String>) -> zyris::Result<Greeting>;
}
```

Doc comments become the tool and field descriptions a model reads, so write them for the model.

## Running the reference node

```bash
cargo run -p zyris-hello
```

With nothing configured it enrolls itself against `attacca.cc`: it prints an 8-character code, you
type that into Attacca on whatever device has a browser, and it connects. Point it elsewhere with
`ZYRIS_SERVER_URL`. See [`crates/zyris-hello/README.md`](crates/zyris-hello/README.md) for the full
configuration table and what to copy.

### With a screen and a keyboard

```bash
nix develop            # or install the packages listed in flake.nix
cargo run -p zyris-hello --features desktop
```

That adds `screen_capture` and `input` to what the node announces, so an agent can see the display
and drive it. Both are addressed the same way: a screenshot names a display and comes back in
display-local pixels, and `input.move_to` names a display and takes them, so a coordinate read off
one goes straight into the other on any number of monitors. The feature is off by default because
it links against the display stack — X11,
Wayland, PipeWire, D-Bus — and a headless node should not have to build any of that. On Linux the
`flake.nix` devShell provides exactly those libraries, `LIBCLANG_PATH` for `pipewire-sys`'s bindgen
step, and an `LD_LIBRARY_PATH` the test binaries need at runtime. It brings no Rust toolchain; use
`nix develop .#full` if the machine has none.

`input` is announced only when the display server accepts a connection, so running with the feature
on a headless box degrades to a warning rather than a node whose tools all fail.

## Documentation

[`docs/zyris-protocol.md`](docs/zyris-protocol.md) is the normative wire reference: framing,
envelopes, the connection lifecycle, streams and flow control, capabilities, datums, video, and the
close codes. Read `zyris-hello` first; read the spec when you need to know exactly what the bytes
mean.

## License

MIT or Apache-2.0, at your option.
