# zyris-p2p

A transport that carries [Zyris](https://github.com/attacca-cc/zyris-protocol) over a direct
node-to-node connection, built on [iroh](https://crates.io/crates/iroh) QUIC.

The usual Zyris connection is a node and a server over one websocket. This one is two nodes talking
to each other with no server in the middle — the server is still how they find each other, but not
how the bytes travel. That is what makes a file transfer between two of an account's own machines
cost the server nothing.

Peer identity is trust-on-first-use: `fingerprint` renders a key as something a human can compare
out of band, `TofuStore` remembers what was accepted, and a `PeerConfirmer` decides what to do the
first time. Key material is written `0600` by `key::load_or_create` rather than left to the caller.

**`iroh` is re-exported from this crate's root and is therefore part of its public API.** An iroh
major release is a breaking release here.

Re-exported as `zyris::p2p` when [`zyris`](https://crates.io/crates/zyris) is built with the `p2p`
feature.

## License

MIT or Apache-2.0, at your option.
