# zyris-core

The [Zyris](https://github.com/attacca-cc/zyris-protocol) node runtime and client: the connection
state machine, the transports, capability announce and accept, the dial-and-reconnect loop, and
RFC 8628 device-grant enrollment.

**Most nodes should depend on [`zyris`](https://crates.io/crates/zyris) instead.** That crate
re-exports everything here at its root and adds the layers above it — `zyris::caps`,
`zyris::p2p` — behind features. `zyris::Node` and
`zyris_core::Node` are one type, so nothing is lost either way.

This crate exists under its own name because every layer above the runtime depends on the runtime,
and Cargo has no cycles: `zyris-caps` and `zyris-capkit` cannot depend on a `zyris` that depends on
them. Depend on `zyris-core` directly when you are writing something in that position — a capability
catalogue or an implementation of one — or when you want the runtime and nothing else.

## Features

- `client` *(default)* — the websocket dialer, `Node::connect`, and the reconnect loop behind
  `Link`.
- `hostname` *(default)* — name the node after the machine it runs on.
- `enroll` — self-registration over the device grant, and the `Account` layer that mints node
  tokens from what it issues. **Turns on `tls-ring`**: it is an HTTPS flow, and `reqwest` on
  `rustls-no-provider` panics while building its client rather than on a request, so leaving
  the choice open would move a panic somewhere with even less context than a dial. Where the credential is stored is the caller's decision, not this
  crate's: it is handed back as a value and taken back as one.
- `tls-ring` / `tls-aws-lc` — the TLS provider rustls negotiates `wss://` with. Exactly one, and
  neither is default: both compile C (`ring` wants `cc`, `aws-lc-rs` wants `cmake` too off its
  nine named targets), and `cargo add` has to stay buildable where there is no toolchain.
  Everything short of dialling works without one. Rustls reads these features and *panics* when
  they name none — deferred to the first connection, with nothing at compile time to warn you —
  so this crate asks first and answers `ConnectError::NoTlsProvider` before opening a socket.
  When both are on, `aws-lc-rs` wins: nobody turns it on by accident. An application that calls
  `CryptoProvider::install_default` itself is believed and needs neither.
- `axum` — serve the protocol from an axum route.
- `testing` — in-process duplex connections for tests.

## License

MIT or Apache-2.0, at your option.
