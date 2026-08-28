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

Nothing here is on crates.io yet, so take it by git:

```toml
[dependencies]
zyris = { git = "https://github.com/attacca-cc/zyris-protocol", features = ["caps", "enroll"] }
```

**Implementations are separate crates, added one at a time.** The catalogue says what a `terminal`
or a `file_io` *is*; what a node offers is the node's own decision, so nothing above pulls an
implementation in. Add the ones you want — `zyris-fs`, `zyris-terminal`, `zyris-transfer`,
`zyris-screen` — the way `ratatui-crossterm` is added beside `ratatui`. Only `zyris-input` is
git-only, and only because of the `enigo` fork it pins.

`default` is a node that can dial and name itself, and costs what `zyris-core` alone costs. `caps`
adds the standard capability declarations; `enroll` adds the 8-character-code flow and the account
layer below. Neither is on by default, because a node holding a `znt_` token out of a secret
manager needs neither.

```rust
use zyris::{Node, NodeKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `HelloServer` is what `#[zyris::capability]` generated from the trait below, wrapped around
    // your implementation of it. The token is any bearer string — see `examples/hello.rs` for
    // where one comes from if you do not have one yet.
    let link = Node::builder()
        .name(zyris::machine_name())
        .kind(NodeKind::Service)
        .capability(HelloServer(HelloWorld))
        .build()?
        .connect(zyris::DEFAULT_SERVER_URL, std::env::var("ZYRIS_NODE_TOKEN")?)
        .await?;

    // `connect` keeps the link up across drops. `Node::dial` is the single attempt underneath it,
    // for a caller who would rather own the retry loop.
    link.wait_closed().await?;
    Ok(())
}
```

The whole of that — capability, token, connection — is one runnable file:
[`crates/zyris/examples/hello.rs`](crates/zyris/examples/hello.rs).

## The crates

| Crate | Reached as | What it is |
|---|---|---|
| [`zyris`](crates/zyris) | — | The whole stack behind one name. The runtime at the root, everything above it as a module behind a feature. **Depend on this one.** |
| [`zyris-core`](crates/zyris-core) | `zyris::*` | The node runtime and client — connection state machine, transports, capability announce/accept, the dial/reconnect loop, and device-grant enrollment. |
| [`zyris-proto`](crates/zyris-proto) | `zyris::proto` | Wire types only: envelopes, frames, the handshake, datums and blobs. No I/O, no async. |
| [`zyris-macros`](crates/zyris-macros) | `zyris::capability` | The `#[zyris::capability]` proc-macro. Not a direct dependency. |
| [`zyris-caps`](crates/zyris-caps) | `zyris::caps` | The standard capability catalogue — `terminal`, `file_io`, `input`, `screen_capture`, `browser_chrome`, `file_transfer`. Declarations only: no tokio, no OS dependencies, cheap for a client to depend on. |
| [`zyris-fs`](crates/zyris-fs) | — | `LocalFileIo`, the reference `file_io` over `tokio::fs`. Pure Rust. |
| [`zyris-terminal`](crates/zyris-terminal) | — | `PtyTerminal`, the reference `terminal` over `portable-pty`. Pure Rust. |
| [`zyris-transfer`](crates/zyris-transfer) | — | The reference `file_transfer` and `peer_transfer`: inbox, resume, undo, and the peer exchange behind `send`/`listen`. |
| [`zyris-screen`](crates/zyris-screen) | — | `HostScreenCapture`, over `xcap` or `zwlr_screencopy`. Pure Rust off Linux; on Linux it needs the display development packages. |
| [`zyris-input`](crates/zyris-input) | — | `EnigoInput`, keyboard and pointer. **The one crate here that cannot be published**: it pins a git fork of `enigo`, and crates.io refuses a manifest that names a git source. |
| [`zyris-attacca`](crates/zyris-attacca) | — | The `attacca_api` capability: the one surface that runs the other way, announced by the server rather than by a node. Named directly rather than reached through `zyris`: Attacca is one deployment, and the protocol's face does not carry it. |
| [`zyris-p2p`](crates/zyris-p2p) | `zyris::p2p` | Transport that carries Zyris over a direct node-to-node connection. |

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

## Running a node

```bash
cargo run -p zyris --example hello --features enroll
```

With nothing configured it enrolls against `attacca.cc`: it prints an 8-character code, you type
that into Attacca on whatever device has a browser, and it connects. Ask an agent on that account
to call `hello.greet` and the node answers "Hello World". Point it elsewhere with
`ZYRIS_SERVER_URL`, or skip the code entirely with `ZYRIS_NODE_TOKEN`.

That one file is the whole library end to end, which is what makes it worth reading before
anything else here.

### A larger node

[`ridanit-ruma/zyris-hello`](https://github.com/ridanit-ruma/zyris-hello) is the reference for
building a real one: it stores its credential and node token on disk, transfers files directly
between nodes, and announces `screen_capture` and `input` behind a feature so an agent can see a
display and drive it. It used to live here as `crates/zyris-hello` and moved out with its history —
this repository is a library, and a program belongs in one of its own.

## Documentation

[`docs/zyris-protocol.md`](docs/zyris-protocol.md) is the normative wire reference: framing,
envelopes, the connection lifecycle, streams and flow control, capabilities, datums, video, and the
close codes. Read [`examples/hello.rs`](crates/zyris/examples/hello.rs) first; read the spec when
you need to know exactly what the bytes mean.

## License

MIT or Apache-2.0, at your option.
