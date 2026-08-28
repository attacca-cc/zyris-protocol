# zyris-fs

The reference `file_io` implementation for the
[Zyris](https://github.com/attacca-cc/zyris-protocol) protocol: local files, over `tokio::fs`.

[`zyris-caps`](https://crates.io/crates/zyris-caps) declares the `file_io` capability — stat, list,
read, a streaming read, write, edit, remove and mkdir — and says nothing about where the bytes live.
This crate is one answer to it, `LocalFileIo`, backed by the filesystem of the machine the node runs
on.

```rust
use zyris::{Node, NodeKind};
use zyris_caps::FileIoServer;
use zyris_fs::LocalFileIo;

let node = Node::builder()
    .name("fs-node")
    .kind(NodeKind::Service)
    // Relative paths from a caller join this directory.
    .capability(FileIoServer(LocalFileIo::rooted(".")))
    .build()?;
```

**The root is a default, not a jail.** A path with a leading `/` addresses the host filesystem
directly and `..` pops out of the root. That is deliberate — an agent working in a checkout still
has to read a sibling directory sometimes — so a node that needs a boundary puts one in front of
this type, where it can see the program's own policy too.

`read` is unary and returns at most 128 KiB, setting `truncated` when the file continues past what
came back; feeding the next `offset` in walks the rest, and `read_stream` has no such ceiling for a
caller that declared a stream.

Named directly, alongside [`zyris`](https://crates.io/crates/zyris). The face carries the
declarations (`zyris::caps`) and not the implementations, because what a node implements is the
node's own decision.

## License

MIT or Apache-2.0, at your option.
