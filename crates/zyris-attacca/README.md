# zyris-attacca

The `attacca_api` capability: the one Zyris surface that runs the *other* way — announced by an
[Attacca](https://attacca.cc) deployment to its nodes, rather than by a node to the server.

Depend on it to call Attacca back from inside a node: drive agents and sessions, look up the
account's other nodes, and resolve a peer for a direct transfer.

It lives in the [Zyris](https://github.com/attacca-cc/zyris-protocol) repository rather than in
Attacca because a node author needs this declaration and nothing else — not Attacca's domain, its
database, or its message bus.

Re-exported as `zyris::attacca` when [`zyris`](https://crates.io/crates/zyris) is built with the
`attacca` feature.

## License

MIT or Apache-2.0, at your option.
