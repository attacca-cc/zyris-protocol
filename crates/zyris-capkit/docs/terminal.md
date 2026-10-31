# `terminal` feature

The reference implementation of the `terminal` capability declared by
[`zyris-caps`](https://crates.io/crates/zyris-caps), for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol.

One type, `PtyTerminal`. It spawns a shell behind a real pseudo-terminal with `portable-pty`, keeps
that shell's output in a per-session ring buffer, and feeds the same bytes to a `vt100` screen
model — so `read` returns the stream while `screen` renders the grid, and neither consumes the
other. `exec` is the one-shot side: no PTY, a bounded run, and on timeout the whole process tree
goes, not just the shell. `poll` is `exec` repeated until a check passes — the node-side counterpart
of Attacca's `computer_terminal_poll`: an attempt every `interval_secs` until it succeeds or the
budget runs out, every attempt's output kept, and an attempt that hangs killed at the deadline
rather than waited on.

```toml
[dependencies]
zyris = { version = "0.3", features = ["caps"] }
zyris-capkit = { version = "0.3", default-features = false, features = ["terminal"] }
```

```rust
use zyris::{Node, NodeKind};
use zyris_caps::TerminalServer;
use zyris_capkit::PtyTerminal;

let node = Node::builder()
    .name("workstation")
    .kind(NodeKind::Service)
    .capability(TerminalServer(PtyTerminal::rooted("/srv/work")))
    .build()?;
```

Sessions are capped, and one nobody has touched for ten minutes is swept away: an agent stuck in a
loop cannot leave shells running, and neither can a node that lost the wire.

## The root is a default, not a jail

`PtyTerminal::rooted` says where a relative path starts and where `exec` runs. An absolute path
still leaves it, because `zyris_caps::resolve_under` resolves rather than fences. A deployment that
needs a fence enforces one above this crate; see the guard in
[`zyris-code`](https://github.com/attacca-cc/zyris-code) for how the reference client does it.

## License

MIT or Apache-2.0, at your option.
