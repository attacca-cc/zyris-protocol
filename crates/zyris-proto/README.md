# zyris-proto

Wire types for the [Zyris](https://github.com/attacca-cc/zyris-protocol) protocol: envelopes,
frames, the handshake, datums and blobs. **No I/O and no async** — this crate is the shape of the
bytes and nothing else, so a tool that only needs to read or write Zyris frames can depend on it
without pulling in a runtime.

Re-exported as `zyris::proto`, so a node that already depends on
[`zyris`](https://crates.io/crates/zyris) does not need this crate by name.

[`docs/zyris-protocol.md`](https://github.com/attacca-cc/zyris-protocol/blob/main/docs/zyris-protocol.md)
is the normative reference for what these types mean on the wire: framing, the connection lifecycle,
streams and flow control, capabilities, datums, video, and the close codes.

## License

MIT or Apache-2.0, at your option.
