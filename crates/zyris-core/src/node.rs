use std::sync::Arc;
#[cfg(feature = "client")]
use std::sync::OnceLock;
#[cfg(feature = "client")]
use std::time::{Duration, Instant};

#[cfg(feature = "client")]
use futures_util::future::BoxFuture;
#[cfg(feature = "client")]
use tokio::sync::watch;

use crate::capabilities::{Capabilities, CapabilitySet};
use crate::connection::{establish, AcceptOptions, Connection, Role};
#[cfg(feature = "client")]
use crate::error::{ConnectError, TransportError};
use crate::error::Result;
use crate::serve::ServeCapability;
use crate::transport::Transport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Desktop,
    Server,
    Cli,
    Service,
}

impl NodeKind {
    /// The wire spelling, as it travels in `Hello::kind`.
    ///
    /// Public so an acceptor can compare against `NodeKind::Cli.as_str()` rather than a bare
    /// `"cli"` — the two ends of that comparison should not be able to drift apart silently.
    pub fn as_str(&self) -> &'static str {
        match self {
            NodeKind::Desktop => "desktop",
            NodeKind::Server => "server",
            NodeKind::Cli => "cli",
            NodeKind::Service => "service",
        }
    }
}

pub struct Node {
    name: String,
    kind: NodeKind,
    capabilities: Arc<CapabilitySet>,
    #[cfg(feature = "client")]
    on_connect: Option<ConnectHook>,
}

pub struct NodeBuilder {
    name: String,
    kind: NodeKind,
    capabilities: Vec<Arc<dyn ServeCapability>>,
    #[cfg(feature = "client")]
    on_connect: Option<ConnectHook>,
}

impl Node {
    pub fn builder() -> NodeBuilder {
        NodeBuilder {
            name: "zyris-node".to_string(),
            kind: NodeKind::Service,
            capabilities: Vec::new(),
            #[cfg(feature = "client")]
            on_connect: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &NodeKind {
        &self.kind
    }

    /// What this node offers, changeable while it is connected. See [`Capabilities`].
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::new(self.capabilities.clone())
    }

    fn agent(&self) -> String {
        format!(
            "zyris/{} ({}; {})",
            env!("CARGO_PKG_VERSION"),
            self.name,
            self.kind.as_str()
        )
    }

    pub async fn connect_over(&self, transport: impl Transport) -> Result<Connection> {
        establish(
            Box::new(transport),
            Role::Dial { agent: self.agent(), kind: self.kind.as_str().to_string() },
            self.capabilities.clone(),
        )
        .await
    }

    /// Open one connection and hand it back. Nothing is retried and nothing is kept up: when this
    /// connection closes it stays closed, and dialling again is the caller's to decide.
    ///
    /// Both halves of a dial can refuse — the HTTP upgrade and the protocol handshake — and both
    /// report a [`WireError`](crate::WireError). They are classified in one place, by
    /// `From<WireError> for ConnectError`, so the two halves cannot drift into disagreeing about
    /// what a refusal means.
    #[cfg(feature = "client")]
    pub async fn dial(&self, url: &str, token: &str) -> Result<Connection, ConnectError> {
        let transport = crate::transport::ws::connect(url, token).await?;
        Ok(self.connect_over(transport).await?)
    }

    /// Another handle to the same node. The capability set is shared rather than copied, so a
    /// capability dropped while connected stays dropped across the reconnect that follows.
    #[cfg(feature = "client")]
    fn share(&self) -> Node {
        Node {
            name: self.name.clone(),
            kind: self.kind.clone(),
            capabilities: self.capabilities.clone(),
            on_connect: self.on_connect.clone(),
        }
    }

    /// Connect, and stay connected.
    ///
    /// Returns once the first dial has settled: a refusal no retry can fix is this call's error,
    /// and anything else is the link's problem — it backs off and dials again. The link ends when
    /// [`Link::disconnect`] is called, when it is dropped, or when the server refuses this node.
    ///
    /// The token is any bearer string: the `NodeToken` an account minted, or a `znt_` read out of a
    /// secret manager. Requiring a type here would make a caller who never touches the account
    /// layer learn it just to name one.
    #[cfg(feature = "client")]
    pub async fn connect(&self, url: &str, token: impl AsRef<str>) -> Result<Link, ConnectError> {
        let node = self.share();
        let url = url.to_string();
        let token = token.as_ref().to_string();
        let redial: Redial = Arc::new(move || {
            let node = node.share();
            let url = url.clone();
            let token = token.clone();
            Box::pin(async move { node.dial(&url, &token).await })
                as BoxFuture<'static, Result<Connection, ConnectError>>
        });
        Link::start(redial, self.on_connect.clone()).await
    }

    pub async fn accept(
        &self,
        transport: impl Transport,
        options: AcceptOptions,
    ) -> Result<Connection> {
        establish(
            Box::new(transport),
            Role::Accept { options },
            self.capabilities.clone(),
        )
        .await
    }
}

impl NodeBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Name this node after the machine it runs on.
    ///
    /// A machine that has nothing usable to say about itself leaves the current name in place, so
    /// this is safe to chain after `name` as an override and before it as a default.
    #[cfg(feature = "hostname")]
    pub fn name_from_hostname(mut self) -> Self {
        if let Some(name) = crate::machine_name() {
            self.name = name;
        }
        self
    }

    pub fn kind(mut self, kind: NodeKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn capability(mut self, capability: impl ServeCapability) -> Self {
        self.capabilities.push(Arc::new(capability));
        self
    }

    pub fn capability_arc(mut self, capability: Arc<dyn ServeCapability>) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Run once per established connection, concurrently with the connection itself — including
    /// the first one, and again after every reconnect.
    ///
    /// This is the *consume* half of a node: the server announces its own capabilities on the same
    /// websocket, so a node is not only a tool provider. The hook is spawned and its outcome is
    /// ignored — a node whose token lacks a scope should still serve the tools it announced.
    #[cfg(feature = "client")]
    pub fn on_connect<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(Connection) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_connect = Some(Arc::new(move |conn| Box::pin(hook(conn))));
        self
    }

    pub fn build(self) -> Result<Node> {
        let capabilities = CapabilitySet::new();
        for cap in self.capabilities {
            capabilities.push(cap)?;
        }
        Ok(Node {
            name: self.name,
            kind: self.kind,
            capabilities,
            #[cfg(feature = "client")]
            on_connect: self.on_connect,
        })
    }
}

/// A connection that stayed up this long counts as healthy, so its eventual drop restarts the
/// backoff from the bottom. Without this a nightly server restart leaves every node pinned at the
/// ceiling forever.
#[cfg(feature = "client")]
const STABLE_AFTER: Duration = Duration::from_secs(30);
#[cfg(feature = "client")]
const BACKOFF_MIN: Duration = Duration::from_secs(1);
#[cfg(feature = "client")]
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Long enough for a closing frame to reach the server, so it retires the node promptly instead of
/// waiting for the heartbeat to lapse.
#[cfg(feature = "client")]
const CLOSE_GRACE: Duration = Duration::from_millis(200);

/// How the link opens one connection. [`Node::connect`] supplies a websocket dial; a test supplies
/// an in-process duplex, which is what lets the reconnect behaviour be driven without a server.
#[cfg(feature = "client")]
type Redial = Arc<dyn Fn() -> BoxFuture<'static, Result<Connection, ConnectError>> + Send + Sync>;

/// Run once per established connection, concurrently with the connection itself.
#[cfg(feature = "client")]
type ConnectHook = Arc<dyn Fn(Connection) -> BoxFuture<'static, ()> + Send + Sync>;

/// Why a link stopped, in a form that can be handed out more than once.
///
/// `ConnectError` carries a `TransportError` and is not `Clone`, but every caller parked on
/// [`Link::wait_closed`] has to get the same answer — so the link keeps what it needs and builds a
/// fresh error each time it is asked.
#[cfg(feature = "client")]
#[derive(Debug, Clone)]
enum Ending {
    Asked,
    Revoked,
    Unauthorized,
    Version { ours: String, theirs: Option<String> },
    Unreachable(TransportError),
}

#[cfg(feature = "client")]
impl Ending {
    /// Exhaustive on purpose: a new `ConnectError` variant should stop the build here rather than
    /// be quietly folded into one of these.
    fn of(error: ConnectError) -> Ending {
        match error {
            ConnectError::Revoked => Ending::Revoked,
            ConnectError::Unauthorized => Ending::Unauthorized,
            ConnectError::VersionMismatch { ours, theirs } => Ending::Version { ours, theirs },
            ConnectError::Unreachable(inner) => Ending::Unreachable(inner),
        }
    }

    fn into_result(self) -> Result<(), ConnectError> {
        match self {
            Ending::Asked => Ok(()),
            Ending::Revoked => Err(ConnectError::Revoked),
            Ending::Unauthorized => Err(ConnectError::Unauthorized),
            Ending::Version { ours, theirs } => Err(ConnectError::VersionMismatch { ours, theirs }),
            Ending::Unreachable(inner) => Err(ConnectError::Unreachable(inner)),
        }
    }
}

/// Retrying changes none of these: they need a person, a different token, or a different build.
#[cfg(feature = "client")]
fn is_fatal(error: &ConnectError) -> bool {
    match error {
        ConnectError::Revoked
        | ConnectError::Unauthorized
        | ConnectError::VersionMismatch { .. } => true,
        ConnectError::Unreachable(_) => false,
    }
}

/// A connection this crate keeps up.
///
/// Dialling, backing off and dialling again happen in a task the link owns. It ends when
/// [`disconnect`](Link::disconnect) is called, when the link is dropped, or when the server refuses
/// this node in a way no retry can fix.
#[cfg(feature = "client")]
pub struct Link {
    /// Set once. Identity comes from the token — `Hello` carries no node id and the server answers
    /// with one — so a redial with the same token lands on the same node and there is nothing here
    /// for a reconnect to change. Empty until the first connection comes up, which is the window a
    /// caller sees when the very first dial was merely unreachable.
    node_id: Arc<OnceLock<String>>,
    /// Dropped when the link is, which is why letting a `Link` go ends it: the loop's receiver
    /// errors and it reads that the same way it reads being asked.
    stop: watch::Sender<bool>,
    done: Arc<watch::Sender<Option<Ending>>>,
}

#[cfg(feature = "client")]
impl Link {
    /// The node id the server assigned, from the `HelloAck` of the connection this link is on.
    pub fn node_id(&self) -> &str {
        self.node_id.get().map(String::as_str).unwrap_or_default()
    }

    /// Resolves when the link is finished: [`disconnect`](Link::disconnect) was called, or the
    /// server refused this node in a way no retry can fix.
    pub async fn wait_closed(&self) -> Result<(), ConnectError> {
        let mut settled = self.done.subscribe();
        loop {
            let ending = settled.borrow().clone();
            if let Some(ending) = ending {
                return ending.into_result();
            }
            if settled.changed().await.is_err() {
                return Ok(());
            }
        }
    }

    /// Ends the link. Does not reconnect afterwards.
    ///
    /// Returns once the maintaining task has actually finished, so a caller that ends a link and
    /// then exits has given the closing frame its chance to land.
    pub async fn disconnect(self) {
        let _ = self.stop.send(true);
        let mut settled = self.done.subscribe();
        loop {
            let ended = settled.borrow().is_some();
            if ended {
                return;
            }
            if settled.changed().await.is_err() {
                return;
            }
        }
    }

    /// The dial that starts a link. A refusal no retry can fix is the caller's error; anything else
    /// is the link's problem, and it goes on trying.
    async fn start(redial: Redial, on_connect: Option<ConnectHook>) -> Result<Link, ConnectError> {
        let first = match redial().await {
            Ok(conn) => Some(conn),
            Err(error) if is_fatal(&error) => return Err(error),
            Err(error) => {
                tracing::warn!(%error, "connect failed");
                None
            }
        };
        let node_id = Arc::new(OnceLock::new());
        // Recorded here rather than left to the task: the connection is already in hand, and a
        // caller that reads `node_id` on the line after `connect` returned must not race a spawn.
        if let Some(conn) = &first {
            let _ = node_id.set(conn.info().node_id.clone());
        }
        let (stop, stop_rx) = watch::channel(false);
        let (done, _) = watch::channel::<Option<Ending>>(None);
        let done = Arc::new(done);
        tokio::spawn(maintain(redial, on_connect, first, node_id.clone(), stop_rx, done.clone()));
        Ok(Link { node_id, stop, done })
    }
}

/// Keep dialling until the link ends.
///
/// Lifted from the runner this replaces and unchanged in substance: backoff with jitter, a
/// connection that stayed up long enough resets it, and a refusal no retry can fix ends the link
/// rather than hammering a server that has already answered.
#[cfg(feature = "client")]
async fn maintain(
    redial: Redial,
    on_connect: Option<ConnectHook>,
    first: Option<Connection>,
    node_id: Arc<OnceLock<String>>,
    mut stop: watch::Receiver<bool>,
    done: Arc<watch::Sender<Option<Ending>>>,
) {
    let mut backoff = BACKOFF_MIN;
    let mut current = first;
    if current.is_none() {
        // The eager dial already failed. Dialling again this instant would make the first retry
        // the only one that never waited.
        let (widened, asked) = wait_then_widen(backoff, &mut stop).await;
        backoff = widened;
        if asked {
            let _ = done.send(Some(Ending::Asked));
            return;
        }
    }
    loop {
        // `borrow` rather than `borrow_and_update`: the select below is what consumes the change,
        // and marking it seen here would lose a stop that arrived mid-dial.
        if *stop.borrow() {
            let _ = done.send(Some(Ending::Asked));
            return;
        }

        let conn = match current.take() {
            Some(conn) => conn,
            None => match redial().await {
                Ok(conn) => conn,
                Err(error) if is_fatal(&error) => {
                    tracing::warn!(%error, "the server refused this node; not dialling again");
                    let _ = done.send(Some(Ending::of(error)));
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "connect failed");
                    let (widened, asked) = wait_then_widen(backoff, &mut stop).await;
                    backoff = widened;
                    if asked {
                        let _ = done.send(Some(Ending::Asked));
                        return;
                    }
                    continue;
                }
            },
        };

        let _ = node_id.set(conn.info().node_id.clone());
        tracing::info!(
            node_id = %conn.info().node_id,
            conn_id = %conn.info().conn_id,
            "connected"
        );
        if let Some(hook) = &on_connect {
            tokio::spawn(hook(conn.clone()));
        }

        let up = Instant::now();
        let asked = tokio::select! {
            reason = conn.closed() => {
                tracing::warn!(%reason, "disconnected");
                false
            }
            // Errs when the `Link` was dropped, which is the same instruction said a shorter way.
            _ = stop.changed() => true,
        };
        if asked {
            conn.close("node disconnecting");
            tokio::time::sleep(CLOSE_GRACE).await;
            let _ = done.send(Some(Ending::Asked));
            return;
        }

        if up.elapsed() >= STABLE_AFTER {
            backoff = BACKOFF_MIN;
        }
        let (widened, asked) = wait_then_widen(backoff, &mut stop).await;
        backoff = widened;
        if asked {
            let _ = done.send(Some(Ending::Asked));
            return;
        }
    }
}

#[cfg(feature = "client")]
async fn wait_then_widen(backoff: Duration, stop: &mut watch::Receiver<bool>) -> (Duration, bool) {
    let wait = jitter(backoff);
    tracing::info!(seconds = wait.as_secs_f64(), "reconnecting");
    let asked = tokio::select! {
        _ = tokio::time::sleep(wait) => false,
        _ = stop.changed() => true,
    };
    ((backoff * 2).min(BACKOFF_MAX), asked)
}

/// ±20%, so a server restart does not bring every node back in the same instant.
///
/// `RandomState` rather than an `rand` dependency: a fresh one hashes differently on every call,
/// which is the entire requirement here. Backoff jitter does not need a real RNG, and adding one to
/// the default feature set would make every node pay for it.
#[cfg(feature = "client")]
fn jitter(base: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u8(0);
    let factor = 0.8 + (hasher.finish() % 400) as f64 / 1000.0;
    base.mul_f64(factor)
}

#[cfg(all(test, feature = "client"))]
mod link_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures_util::future::BoxFuture;

    use super::*;
    use crate::connection::Connection;

    /// Long enough for a link that was going to do something wrong to have done it, short enough
    /// that a test asserting "nothing happened" is not the slow one in the suite.
    const A_MOMENT: Duration = Duration::from_millis(200);

    /// A dial that hands back an in-process connection and keeps the other end, so a test can drop
    /// a connection the way a server restart does and watch what the link does next.
    fn duplex_redial(served: Arc<Mutex<Vec<Connection>>>, dials: Arc<AtomicUsize>) -> Redial {
        Arc::new(move || {
            let served = served.clone();
            let dials = dials.clone();
            Box::pin(async move {
                dials.fetch_add(1, Ordering::SeqCst);
                let dialer = Node::builder().name("probe").kind(NodeKind::Cli).build().unwrap();
                let acceptor =
                    Node::builder().name("server").kind(NodeKind::Server).build().unwrap();
                let (mine, theirs) = crate::testing::duplex(&dialer, &acceptor)
                    .await
                    .map_err(|e| ConnectError::Unreachable(TransportError::Io(e.to_string())))?;
                served.lock().unwrap().push(theirs);
                Ok(mine)
            }) as BoxFuture<'static, Result<Connection, ConnectError>>
        })
    }

    /// Poll until a condition holds. The backoff floor is a second, so the deadline has to sit
    /// comfortably past it or a slow machine reads as a link that never reconnected.
    async fn wait_until(what: &str, mut settled: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !settled() {
            assert!(tokio::time::Instant::now() < deadline, "the link never {what}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// A token the server has refused is not a token that will work on the next try. This is why
    /// the error is typed at all: the node stops and says so, instead of dialling a server that
    /// has already answered.
    #[tokio::test]
    async fn a_refused_token_stops_the_link_instead_of_dialling_again() {
        let dials = Arc::new(AtomicUsize::new(0));
        let counted = dials.clone();
        let redial: Redial = Arc::new(move || {
            let counted = counted.clone();
            Box::pin(async move {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(ConnectError::Unauthorized)
            }) as BoxFuture<'static, Result<Connection, ConnectError>>
        });

        let error = Link::start(redial, None).await.err().expect("a refusal is not a link");
        assert!(matches!(error, ConnectError::Unauthorized), "got: {error}");

        tokio::time::sleep(A_MOMENT).await;
        assert_eq!(
            dials.load(Ordering::SeqCst),
            1,
            "a refusal is answered once; answering it in a loop is what the type exists to stop"
        );
    }

    /// The node id is the server's answer, not the node's claim — `Hello` has no field for it — so
    /// a caller that wants to name this node in a log has nowhere else to read it.
    #[tokio::test]
    async fn a_link_reports_the_node_id_the_server_handed_back() {
        let served = Arc::new(Mutex::new(Vec::new()));
        let dials = Arc::new(AtomicUsize::new(0));
        let link = Link::start(duplex_redial(served.clone(), dials), None)
            .await
            .expect("an in-process duplex comes up");

        let assigned = served.lock().unwrap()[0].info().node_id.clone();
        assert!(!assigned.is_empty(), "the acceptor assigns one");
        assert_eq!(link.node_id(), assigned, "the link reports what the acceptor assigned");
    }

    /// Everything a connect hook does is per connection: publishing where this node can be
    /// reached, picking up the client for what the server announced back. A reconnect that skipped
    /// it would leave the node connected and useless, which reads as a working node.
    #[tokio::test]
    async fn the_connect_hook_runs_again_on_the_connection_that_replaces_a_dropped_one() {
        let served = Arc::new(Mutex::new(Vec::new()));
        let dials = Arc::new(AtomicUsize::new(0));
        let hooked = Arc::new(AtomicUsize::new(0));
        let counted = hooked.clone();
        let hook: ConnectHook = Arc::new(move |_conn| {
            let counted = counted.clone();
            Box::pin(async move {
                counted.fetch_add(1, Ordering::SeqCst);
            }) as BoxFuture<'static, ()>
        });

        let _link = Link::start(duplex_redial(served.clone(), dials.clone()), Some(hook))
            .await
            .expect("an in-process duplex comes up");
        wait_until("ran the hook on the first connection", || {
            hooked.load(Ordering::SeqCst) == 1
        })
        .await;

        served.lock().unwrap()[0].close("server restarting");
        wait_until("ran the hook again after reconnecting", || {
            hooked.load(Ordering::SeqCst) == 2
        })
        .await;
        assert_eq!(dials.load(Ordering::SeqCst), 2, "and it got there by dialling again");
    }

    /// A server restart drops every connection it was holding. A link that reported itself
    /// finished at that moment would take the node down with the server it was talking to.
    #[tokio::test]
    async fn one_connection_dropping_is_not_the_link_finishing() {
        let served = Arc::new(Mutex::new(Vec::new()));
        let dials = Arc::new(AtomicUsize::new(0));
        let link = Link::start(duplex_redial(served.clone(), dials.clone()), None)
            .await
            .expect("an in-process duplex comes up");

        served.lock().unwrap()[0].close("server restarting");
        wait_until("dialled again", || dials.load(Ordering::SeqCst) == 2).await;

        assert!(
            tokio::time::timeout(A_MOMENT, link.wait_closed()).await.is_err(),
            "a link that reconnected is still up, and has to keep whoever waits on it waiting"
        );
    }

    /// The other half of the same judgement: when redialling will never work, the link stops and
    /// the reason reaches whoever is waiting, rather than being logged into a void while the loop
    /// keeps going.
    #[tokio::test]
    async fn a_link_the_server_disowns_hands_the_reason_to_whoever_waits_for_it() {
        let served = Arc::new(Mutex::new(Vec::new()));
        let dials = Arc::new(AtomicUsize::new(0));
        let live = duplex_redial(served.clone(), dials);
        let attempts = Arc::new(AtomicUsize::new(0));
        let redial: Redial = Arc::new(move || {
            let live = live.clone();
            let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if first {
                    live().await
                } else {
                    Err(ConnectError::Revoked)
                }
            }) as BoxFuture<'static, Result<Connection, ConnectError>>
        });

        let link = Link::start(redial, None).await.expect("the first dial comes up");
        served.lock().unwrap()[0].close("credential revoked");

        let ended = tokio::time::timeout(Duration::from_secs(10), link.wait_closed())
            .await
            .expect("a revoked credential ends the link rather than looping on it");
        assert!(matches!(ended, Err(ConnectError::Revoked)), "got: {ended:?}");
    }
    /// Ending a link is not the same as walking away from one. The server has to see the
    /// connection go, or it holds the node's registry slot until the heartbeat lapses — and
    /// nothing may dial afterwards, or "disconnect" is a word for a pause.
    #[tokio::test]
    async fn ending_a_link_closes_the_connection_it_was_keeping_up() {
        let served = Arc::new(Mutex::new(Vec::new()));
        let dials = Arc::new(AtomicUsize::new(0));
        let link = Link::start(duplex_redial(served.clone(), dials.clone()), None)
            .await
            .expect("an in-process duplex comes up");

        assert!(
            tokio::time::timeout(A_MOMENT, link.wait_closed()).await.is_err(),
            "a link that is still connected has not finished"
        );

        tokio::time::timeout(Duration::from_secs(5), link.disconnect())
            .await
            .expect("disconnect returns once the link has actually ended");

        let peer = served.lock().unwrap().remove(0);
        tokio::time::timeout(Duration::from_secs(5), peer.closed())
            .await
            .expect("the other end has to see the connection go, not wait for a heartbeat to lapse");
        assert_eq!(dials.load(Ordering::SeqCst), 1, "an ended link does not dial again");
    }
}
