# zyris

The [Zyris](https://github.com/attacca-cc/zyris-protocol) protocol stack behind one name.

A **Zyris node** is any machine that dials a server over one websocket, announces typed
**capabilities** — named, versioned sets of tools with JSON-Schema'd arguments — and, on the same
connection, consumes the capabilities the server announces back. The protocol is
direction-symmetric: once the handshake is done there is no client role and no server role, and
either peer may call the other, open streams, or change what it offers mid-session.

```toml
[dependencies]
zyris = { version = "0.2", features = ["full"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
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

There is no exit code, no signal handler and no config directory in that snippet, and none in
this crate either. Enrollment hands the code back as a value, `Account` hands rotations back
through a callback, and when the process stops is the program's decision.

## What this crate is

One dependency for the whole stack. The runtime is re-exported at the root, and each layer above it
is a module behind a feature:

| Path | Feature | Crate | What it is |
|---|---|---|---|
| `zyris::*` | — | [`zyris-core`](https://crates.io/crates/zyris-core) | The runtime: connection state machine, transports, announce/accept, the dial-and-reconnect loop, device-grant enrollment. |
| `zyris::proto` | — | [`zyris-proto`](https://crates.io/crates/zyris-proto) | Wire types only. No I/O, no async. |
| `zyris::caps` | `caps` | [`zyris-caps`](https://crates.io/crates/zyris-caps) | The standard capability catalogue. Declarations only. |
| `zyris::p2p` | `p2p` | [`zyris-p2p`](https://crates.io/crates/zyris-p2p) | Transport that carries Zyris directly between two nodes. |

Each is also a crate of its own. Depending on one directly *and* on this one is not a mistake and
links nothing twice — the module here **is** that crate.

## Features

`default = ["client", "hostname"]` — the same default as `zyris-core`, so `cargo add zyris` costs
what depending on the runtime alone does.

- **Runtime:** `client`, `enroll`, `hostname`, `axum`, `testing`
- **Stack:** `caps`, `p2p`, and `full` for both plus `enroll`

**Implementations are not here.** The repository's `zyris-capkit` has reference ones and is not
published: a node decides what it offers, and a crate that pins a git fork of `enigo` could not go
to crates.io in any case. Depend on it by git when you want them.

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

## Where to start

[`examples/hello.rs`](examples/hello.rs) is the whole library in one runnable file: declare a
capability, take a node, serve it, answer "Hello World" when a model calls it.

```bash
cargo run -p zyris --example hello --features enroll
```

For a node with more in it — a credential stored on disk, file transfer, screen capture and input
— read [`ridanit-ruma/zyris-hello`](https://github.com/ridanit-ruma/zyris-hello).

[`docs/zyris-protocol.md`](https://github.com/attacca-cc/zyris-protocol/blob/main/docs/zyris-protocol.md)
is the normative wire reference.

## License

MIT or Apache-2.0, at your option.
