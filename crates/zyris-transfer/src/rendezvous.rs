//! The handle both network halves ask where a peer is.
//!
//! It lives in its own module because **both of them need it and neither owns it**. The sending
//! side holds one to look a peer up before dialing; the receiving side implements
//! `listen::PeerDirectory` on it so the accept loop can ask whose nodes these are — and that
//! module is behind its own feature, so this says the name rather than linking it. It used to sit
//! in `send.rs`, which made `listen` reach across into it — so the
//! `listen` feature did not compile on its own, however plainly the manifest said the two were
//! independent. This module is compiled whenever either feature is on, and neither reaches into
//! the other any more.

use std::sync::{Arc, RwLock};

use zyris_attacca::AttaccaApiClient;

/// The client this node asks Attacca where a peer is — **replaceable, because the connection it
/// belongs to is**.
///
/// It used to be a `OnceLock`, on the reasoning that a connection has one client and a slot that
/// can be overwritten is a slot that can be swapped. The first half is true and the second is the
/// wrong conclusion: a node's websocket drops and comes back — a laptop sleeps, a server rolls, a
/// network blips — and the `Link` reconnects and calls `set_api` again with a client for the new
/// connection. A write-once slot ignores that call and keeps the client bound to the dead one, so
/// every lookup after the first disconnect fails with `connection lost`, for good, on a node that
/// otherwise looks perfectly healthy. Observed live: a send worked, the socket reset once, and
/// every send after it failed identically until the process was restarted.
///
/// Nothing outside this node can reach `set`. The client always comes from a connection the node
/// itself established, so "could be overwritten" was never a way in — only a way to stay broken.
///
/// Cloning shares the slot, which is what lets one client serve both the send side and
/// `listen::serve_peers`'s directory.
#[derive(Clone, Default)]
pub struct Rendezvous(Arc<RwLock<Option<Arc<AttaccaApiClient>>>>);

impl Rendezvous {
    /// Replaces the client. Call on every connect.
    pub fn set(&self, api: AttaccaApiClient) {
        // A poisoned lock here would mean a panic while swapping a client, which is not a reason to
        // take the node down with it — the worst case is one stale client, which the next connect
        // replaces anyway.
        if let Ok(mut slot) = self.0.write() {
            *slot = Some(Arc::new(api));
        }
    }

    /// The current client, or `None` before the first connect.
    ///
    /// Returns an owned handle rather than a guard, so no caller can hold the lock across an
    /// `await` and block the next reconnect from installing its client.
    pub fn get(&self) -> Option<Arc<AttaccaApiClient>> {
        self.0.read().ok().and_then(|slot| slot.clone())
    }
}
