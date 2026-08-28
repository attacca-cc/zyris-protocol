//! The standard Zyris capability catalogue: declarations only.
//!
//! Each module holds one `#[zyris::capability]` trait and the types it speaks in. The macro turns
//! each into a descriptor, a `…Server<T>` wrapper a node registers, and a `…Client` a peer calls
//! through — so a client crate and a server crate agree on the wire by depending on this and
//! nothing else.
//!
//! There is deliberately no implementation here, and no dependency that touches an operating
//! system. The reference implementations are separate crates — `zyris-fs`, `zyris-terminal`,
//! `zyris-transfer`, `zyris-screen`, `zyris-input` — and a node adds whichever it wants.
//!
//! A few items here are not declarations but *contract*: `resolve_under`, `Displays`,
//! `no_such_display`, `display_scale`. They are what two independent implementations of one
//! declaration have to agree on, and they live here for the same reason the declarations do —
//! duplicated per implementation they would drift, and the drift would be in what a request
//! means rather than in how it is served. All four are std-only or `WireError`, so a client
//! that never implements anything still pays nothing for them.

/// Resolving a caller-supplied path against a node's root, which every capability that takes a
/// path has to do the same way. Two implementations that disagreed about what `../` means would
/// disagree about what a request *said*, which is the contract, not the behaviour.
pub mod path;
pub use path::resolve_under;

pub mod browser;
pub mod file_io;
pub mod file_transfer;
pub mod input;
pub mod peer_transfer;
pub mod screen;
pub mod terminal;

pub use browser::{browser_chrome_capability, BrowserChrome, BrowserChromeClient, BrowserChromeServer};
pub use file_io::{
    file_io_capability, DirEntry, FileEdit, FileIo, FileIoClient, FileIoServer, FileRead, FileStat,
};
pub use file_transfer::{
    file_transfer_capability, FileTransfer, FileTransferClient, FileTransferServer, InboxEntry,
    SendReceipt,
};
pub use input::{input_capability, Input, InputClient, InputServer, MouseButton};
pub use peer_transfer::{
    peer_transfer_capability, PeerTransfer, PeerTransferClient, PeerTransferServer, PullHead,
    TransferDone, TransferOffer,
};
pub use screen::{
    display_scale, no_such_display, screen_capture_capability, Display, Displays, ImageFormat,
    Region, ScreenCapture, ScreenCaptureClient, ScreenCaptureServer,
};
pub use terminal::{
    terminal_capability, ExecOutput, PtyChunk, PtyId, PtyOpened, PtyRead, PtyScreen, Settle,
    Terminal, TerminalClient, TerminalServer,
};

/// What `#[zyris(limit = ...)]` puts on a descriptor.
///
/// The macro is a proc macro and cannot test its own output, so the check lives next to the
/// capabilities that use it: declare one, take its descriptor, and read the field back.
#[cfg(test)]
mod call_limit_tests {
    use zyris::{CallLimit, Datum};

    /// A tool of each kind. The doc comments are load-bearing elsewhere, so they are here too.
    #[zyris::capability(name = "slow", version = 1)]
    pub trait Slow {
        /// Returns at once.
        async fn ping(&self) -> zyris::Result<Datum>;
        /// Takes a while, but bounded.
        #[zyris(limit = 600)]
        async fn build(&self) -> zyris::Result<Datum>;
        /// Takes as long as it takes.
        #[zyris(limit = false)]
        async fn deploy(&self) -> zyris::Result<Datum>;
    }

    #[test]
    fn a_tool_declares_what_it_asks_of_a_callers_clock() {
        let descriptor = slow_capability();
        let limit = |name: &str| descriptor.tool(name).expect("tool is announced").call_limit;

        assert_eq!(limit("ping"), None, "saying nothing asks for the caller's default");
        assert_eq!(limit("build"), Some(CallLimit::Secs(600)));
        assert_eq!(
            limit("deploy"),
            Some(CallLimit::Unlimited),
            "`limit = false` is the whole way to ask for no clock at all"
        );
    }

    /// The attribute has to compose with the transfer mode rather than replace it — they are set
    /// by the same `#[zyris(..)]` attribute, and the parser now reads two shapes to get there.
    #[zyris::capability(name = "slow_stream", version = 1)]
    pub trait SlowStream {
        /// Streams for as long as it needs to.
        #[zyris(uni_stream, limit = false)]
        async fn watch(&self) -> zyris::Result<zyris::Streaming<Datum, Datum>>;
    }

    #[test]
    fn a_streaming_tool_can_carry_a_limit_too() {
        let descriptor = slow_stream_capability();
        let watch = descriptor.tool("watch").expect("tool is announced");
        assert_eq!(watch.transfer, zyris::Transfer::UniStream, "the mode still applies");
        assert_eq!(watch.call_limit, Some(CallLimit::Unlimited));
    }
}
