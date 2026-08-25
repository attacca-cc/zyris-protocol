//! The Zyris protocol stack behind one name.
//!
//! A **Zyris node** dials a server over one websocket, announces typed **capabilities** — named,
//! versioned sets of tools with JSON-Schema'd arguments — and, on the same connection, consumes the
//! capabilities the server announces back. This crate is the whole stack behind one dependency:
//!
//! ```toml
//! zyris = { version = "0.2", features = ["full"] }
//! ```
//!
//! ```no_run
//! use zyris::{Node, NodeKind};
//!
//! # async fn dial() -> Result<(), Box<dyn std::error::Error>> {
//! let node = Node::builder().name("my node").kind(NodeKind::Service).build()?;
//!
//! // `connect` keeps the link up across drops; `Node::dial` is the single attempt underneath it.
//! let link = node.connect("wss://attacca.cc/api/zyris/v1/ws", "znt_the_node_token").await?;
//! link.wait_closed().await?;
//! # Ok(())
//! # }
//! ```
//!
//! That is the half a node does on every start. The other half — enrolling once, keeping the
//! account credential, and minting a node token from it — is spelled out end to end on
//! `zyris::enroll`, behind the `enroll` feature (so it is not a link here: under default
//! features that item does not exist). Capability declarations live in `zyris::caps`, behind
//! `caps`; what a node *implements* is the node's own decision, which is why nothing published
//! here makes it.
//!
//! Implementations of those declarations are not here on purpose: what a node offers is the
//! node's decision. The repository carries reference ones, deliberately unpublished — see
//! <https://github.com/attacca-cc/zyris-protocol>.
//!
//! # What is where
//!
//! The runtime — connection state machine, transports, capability announce and accept, the
//! dial/reconnect loop, device-grant enrollment, the account layer — is re-exported at this
//! crate's root, so `Node`, `Connection`, `Result` and everything beside them are named
//! directly. The layers above the runtime each get a module, and each is behind a feature:
//!
//! | Module | Feature | What it is |
//! |---|---|---|
//! | `caps` | `caps` | The standard capability catalogue. Declarations only. |
//! | `attacca` | `attacca` | The `attacca_api` capability — the one that runs the other way. |
//! | `p2p` | `p2p` | Transport that carries Zyris directly between two nodes. |
//!
//! Each is also a crate of its own (`zyris-caps`, `zyris-attacca`, …). Depending on one directly
//! and on this one at the same time is not a mistake and does not link anything twice: the
//! module here *is* that crate.
//!
//! # Why the runtime lives in `zyris-core`
//!
//! Every crate above the runtime depends on the runtime, so the runtime cannot depend on any of
//! them — Cargo has no cycles. The runtime therefore ships as `zyris-core` and this crate
//! re-exports it whole. Nothing about that is visible in a path: `zyris::Node` and
//! `zyris_core::Node` are one type, and a capability written against either serves the other.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub use zyris_core::*;

/// The runtime, under its own name.
///
/// Reach for this only when a macro or a generated path has to name the crate the runtime actually
/// ships as. For everything else the re-exports at this crate's root are the same items.
pub use zyris_core;

/// The standard capability catalogue: `terminal`, `file_io`, `input`, `screen_capture`,
/// `browser_chrome`. Declarations only — no tokio, no OS dependencies.
#[cfg(feature = "caps")]
#[cfg_attr(docsrs, doc(cfg(feature = "caps")))]
pub use zyris_caps as caps;

/// The `attacca_api` capability: the surface an Attacca deployment announces to its nodes, rather
/// than the other way round. Depend on it to call Attacca back.
#[cfg(feature = "attacca")]
#[cfg_attr(docsrs, doc(cfg(feature = "attacca")))]
pub use zyris_attacca as attacca;

/// Transport that carries Zyris over a direct node-to-node connection.
#[cfg(feature = "p2p")]
#[cfg_attr(docsrs, doc(cfg(feature = "p2p")))]
pub use zyris_p2p as p2p;
