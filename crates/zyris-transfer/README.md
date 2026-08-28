# zyris-transfer

The reference file-transfer implementation for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol.

[`zyris-caps`](https://crates.io/crates/zyris-caps) declares two capabilities here and says nothing
about how either is served. `peer_transfer` is the exchange two nodes have with each other;
`file_transfer` is the surface an agent calls to start one and to read what has arrived. This crate
is one answer to both — `LocalPeerTransfer` for the exchange, `LocalFileTransfer` for the surface.

```toml
[dependencies]
zyris = { version = "0.2", features = ["caps"] }
zyris-transfer = "0.2"
```

```rust
use zyris::{Node, NodeKind};
use zyris_caps::PeerTransferServer;
use zyris_transfer::{LocalPeerTransfer, TransferConfig};

let config = TransferConfig {
    inbox: "/srv/inbox".into(),
    undo: "/srv/undo".into(),
    ..TransferConfig::default()
};
// The peer's own name, which decides which subdirectory of the inbox its files land in.
let receiving = LocalPeerTransfer::receiver_pending(config, "laptop".to_string());
let node = Node::builder()
    .name("workstation")
    .kind(NodeKind::Service)
    .capability(PeerTransferServer(receiving))
    .build()?;
```

## There is nobody to ask

An agent calls a tool and a file lands on someone else's machine. No human confirms the receiving
side, so **the path jail, the size limit and the audit log are the only defenses there are.** A
proposed name is washed down to one path component, the resolved destination is checked against the
real filesystem and refused if any component of it is a symlink, an existing file is moved aside
before it is replaced, and one line per transfer is appended to a log — because in a flow with no
confirmation that log is the only way to find out afterwards what happened.

A transfer resumes rather than restarts. The same arguments name the same transfer, the bytes
already received are on the far side as a `.part`, and calling again picks up from there; one writer
per transfer is enforced, so a retry that overlaps its own first call is turned away instead of
appending into the same file behind it.

## Features

Everything above is unconditional. It touches nothing but the filesystem, so it costs no transport
dependencies, and it alone is enough to drive a whole exchange over `zyris::testing::duplex`.

| Feature | What it adds |
|---|---|
| `send` | `file_transfer.send_to`: looking a peer up, pinning its key, and dialing it |
| `listen` | Accepting peer connections, and deciding whose they are |

**Both are off by default and each stands on its own.** The two directions are separately useful — a
node can take deliveries without carrying the tool surface that makes them.

Turning either one on reaches [`zyris-p2p`](https://crates.io/crates/zyris-p2p), and through it
iroh, whose default TLS backend is `ring` — a C build. Cargo features are additive across a
workspace, so switching this on for one member switches it on for every build of this crate in that
graph.

## License

MIT or Apache-2.0, at your option.
