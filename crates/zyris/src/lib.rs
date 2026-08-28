//! The Zyris protocol stack behind one name.
//!
//! A **Zyris node** dials a server over one websocket, announces typed **capabilities** — named,
//! versioned sets of tools with JSON-Schema'd arguments — and, on the same connection, consumes the
//! capabilities the server announces back. This crate is the whole stack behind one dependency:
//!
//! ```toml
//! zyris = { version = "0.2", features = ["full", "tls-ring"] }
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
//! # Dialling needs a TLS provider, and it is yours to name
//!
//! `wss://` is TLS, rustls does TLS, and rustls has no cipher suite of its own: it reads its
//! crate features and, when they name no provider, it panics on the first connection. That
//! panic is deferred all the way to runtime — there is no `compile_error!` for it — so a build
//! with none is green, starts, announces, and dies the moment it dials.
//!
//! So neither provider is on by default and this crate answers the question itself. Turn on
//! exactly one:
//!
//! | Feature | Provider | What it costs to build |
//! |---|---|---|
//! | `tls-ring` | `ring` | a working `cc` |
//! | `tls-aws-lc` | `aws-lc-rs` | a working `cc`, and `cmake` off its nine named targets |
//!
//! Both compile C, which is why neither is default: `cargo add zyris` stays buildable on a
//! machine with no toolchain, and everything short of reaching the network works there —
//! declaring capabilities, serving them over another transport, the wire types. A build that
//! skipped this step and dials anyway is refused by [`ConnectError::NoTlsProvider`], before a
//! socket is opened, naming both features. An application that installs its own provider with
//! `rustls::crypto::CryptoProvider::install_default` is believed and needs neither.
//!
//! **`enroll` names `tls-ring` for you**, because there is no such thing as enrolling without TLS:
//! it is an HTTPS device-grant flow, and `reqwest` panics while *building its client* rather than
//! on a request when no provider is installed. Adding `tls-aws-lc` alongside overrides the choice.
//! `p2p` reaches a provider too, through iroh — that one is a property of wanting QUIC rather than
//! something this crate decided.
//!
//! That is the half a node does on every start. The other half — enrolling once, keeping the
//! account credential, and minting a node token from it — is spelled out end to end on
//! `zyris::enroll`, behind the `enroll` feature (so it is not a link here: under default
//! features that item does not exist). Capability declarations live in `zyris::caps`, behind
//! `caps`; what a node *implements* is the node's own decision, which is why nothing published
//! here makes it.
//!
//! Implementations of those declarations are not here on purpose: what a node offers is the
//! node's decision. Reference ones are separate crates, added beside this one as you want them —
//! `zyris-fs`, `zyris-terminal`, `zyris-transfer`, `zyris-screen`. `zyris-input` is the exception
//! and stays git-only: it pins a fork of `enigo`, and crates.io refuses a manifest naming a git
//! source. See <https://github.com/attacca-cc/zyris-protocol>.
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

/// Transport that carries Zyris over a direct node-to-node connection.
#[cfg(feature = "p2p")]
#[cfg_attr(docsrs, doc(cfg(feature = "p2p")))]
pub use zyris_p2p as p2p;
