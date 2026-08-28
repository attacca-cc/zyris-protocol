//! The reference implementation of node-to-node file transfer for Zyris.
//!
//! [`zyris_caps`] declares two capabilities here and says nothing about how either is served.
//! `peer_transfer` is the exchange two nodes have with each other; `file_transfer` is the surface
//! an agent calls to start one and to read what has arrived. This crate is one answer to both.
//!
//! There is no human confirmation on the receiving side — an agent calls a tool and a file lands
//! on someone else's machine. **The path jail, the size limit and the audit log are the only
//! defenses there are.** This crate splits those between its parts.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`name`] | Proposed name → a single path component. Never touches the filesystem |
//! | [`inbox`] | Choosing the destination and checking the real path. Rejects symlinks |
//! | [`undo`] | Moves the original aside before overwriting |
//! | [`audit`] | Writes one line per transfer |
//! | [`peer`] | Wires the above together into `peer_transfer` |
//! | `send` | The other direction — the `file_transfer` surface an agent calls |
//! | `listen` | Accepting peer connections, and deciding whose they are |
//! | `rendezvous` | The Attacca client both of those ask where a peer is |
//!
//! `name` is kept apart from `inbox` on purpose — sanitizing is a pure decision, so dozens of
//! cases run as one table test in an instant. Merged in, every one of those tests would need a
//! `tempfile`.
//!
//! # Features
//!
//! Everything above the line is unconditional: it touches nothing but the filesystem, so it costs
//! no transport dependencies, and it alone is enough to drive a whole exchange over
//! `zyris::testing::duplex`.
//!
//! | Feature | What it adds |
//! |---|---|
//! | `send` | `file_transfer.send_to`: the rendezvous client and the iroh dial |
//! | `listen` | Accepting peer connections: the rendezvous, and iroh to accept at all |
//!
//! **Both are off by default and each stands on its own.** The two directions are separately
//! useful — a node can take deliveries without carrying the tool surface that makes them, and a
//! node can make them without listening. `rendezvous` is compiled for either, which is what keeps
//! that claim true: it used to live inside `send`, so `listen` reached across into it and did not
//! compile alone, however plainly this said otherwise.
//!
//! Turning either on reaches iroh, and through it a `ring` build that needs a C toolchain. See
//! this crate's `Cargo.toml`.
//!
//! ```no_run
//! use zyris::{Node, NodeKind};
//! use zyris_caps::PeerTransferServer;
//! use zyris_transfer::{LocalPeerTransfer, TransferConfig};
//!
//! # fn serve() -> zyris::Result<Node> {
//! let config = TransferConfig {
//!     inbox: "/srv/inbox".into(),
//!     undo: "/srv/undo".into(),
//!     ..TransferConfig::default()
//! };
//! // The peer's own name, which decides which subdirectory of the inbox its files land in.
//! let receiving = LocalPeerTransfer::receiver_pending(config, "laptop".to_string());
//! let node = Node::builder()
//!     .name("workstation")
//!     .kind(NodeKind::Service)
//!     .capability(PeerTransferServer(receiving))
//!     .build()?;
//! # Ok(node)
//! # }
//! ```

pub mod audit;
pub mod inbox;
#[cfg(feature = "listen")]
pub mod listen;
pub mod name;
pub mod peer;
#[cfg(any(feature = "send", feature = "listen"))]
pub mod rendezvous;
#[cfg(feature = "send")]
pub mod send;
pub mod undo;

pub use audit::{Audit, AuditLine};
pub use inbox::{Inbox, InboxError};
#[cfg(feature = "listen")]
pub use listen::{
    serve_peers, PeerCache, PeerDirectory, DEFAULT_DIRECTORY_TTL, DEFAULT_REFRESH_INTERVAL,
};
pub use name::safe_name;
pub use peer::{InFlight, LocalPeerTransfer, TRANSFER_IN_FLIGHT, TransferConfig, part_path};
#[cfg(any(feature = "send", feature = "listen"))]
pub use rendezvous::Rendezvous;
#[cfg(feature = "send")]
pub use send::{
    FileTransferConfig, IrohPeerLink, LocalFileTransfer, PeerLink, PeerSession,
    DEFAULT_WIRE_DEADLINE,
};
pub use undo::UndoStore;
