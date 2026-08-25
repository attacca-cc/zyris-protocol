# zyris-caps

The standard capability catalogue for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol — `terminal`, `file_io`, `input`,
`screen_capture`, `browser_chrome`, `file_transfer` and `peer_transfer`.

**Declarations only.** No tokio, no OS dependencies, nothing that touches a disk or a display: this
crate is the trait, the descriptor and the generated client for each capability, so a peer that only
*calls* these tools is cheap to build.

For implementations of the same catalogue, see
[`zyris-capkit`](https://crates.io/crates/zyris-capkit).

Re-exported as `zyris::caps` when [`zyris`](https://crates.io/crates/zyris) is built with the `caps`
feature; a node that depends on that crate does not need this one by name.

## License

MIT or Apache-2.0, at your option.
