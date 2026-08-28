//! The reference `file_io` implementation for
//! [Zyris](https://github.com/attacca-cc/zyris-protocol): local files, over `tokio::fs`.
//!
//! [`zyris_caps::FileIo`] is the contract — stat, list, read, a streaming read, write, edit,
//! remove and mkdir — and says nothing about where the bytes live. This crate is one answer to it,
//! [`LocalFileIo`], backed by the filesystem of the machine the node runs on. A node that wants
//! different storage behind the same tools implements the trait itself and never depends on this.
//!
//! ```no_run
//! use zyris::{Node, NodeKind};
//! use zyris_caps::FileIoServer;
//! use zyris_fs::LocalFileIo;
//!
//! # fn serve() -> zyris::Result<Node> {
//! let node = Node::builder()
//!     .name("fs-node")
//!     .kind(NodeKind::Service)
//!     // Relative paths from a caller join this directory.
//!     .capability(FileIoServer(LocalFileIo::rooted(".")))
//!     .build()?;
//! # Ok(node)
//! # }
//! ```
//!
//! # The root is a default, not a jail
//!
//! [`LocalFileIo::rooted`] takes the directory a caller's relative paths join. It is not a
//! boundary: a path with a leading `/` addresses the host filesystem directly and `..` pops out of
//! the root, exactly as [`zyris_caps::resolve_under`] describes. That is deliberate — an agent
//! working in a checkout still has to read `/etc/hostname` or a sibling directory sometimes, and a
//! limit that some callers need is not the same as one every caller gets.
//!
//! A node that does need a boundary puts one in front of this type, where it can also see the
//! program's own policy. Enforcing it is not this crate's job, and pretending otherwise would be
//! worse than saying so.
//!
//! # Reads are bounded, and say when they are
//!
//! [`FileIo::read`](zyris_caps::FileIo::read) is unary, so its whole result has to fit in one
//! response; it returns at most 128 KiB and sets `truncated` when the file continues past what it
//! returned. Feeding the next `offset` back in walks the rest, and
//! [`FileIo::read_stream`](zyris_caps::FileIo::read_stream) has no such ceiling for a caller that
//! declared a stream.

mod file_io;
pub use file_io::LocalFileIo;
