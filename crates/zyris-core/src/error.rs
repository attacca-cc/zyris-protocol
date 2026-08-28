pub use zyris_proto::{ErrorCode, WireError};

pub type Error = WireError;
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,
    #[error("transport error: {0}")]
    Io(String),
}

impl From<TransportError> for WireError {
    fn from(err: TransportError) -> Self {
        match err {
            TransportError::Closed => WireError::connection_lost(),
            TransportError::Io(msg) => {
                WireError::new(ErrorCode::ConnectionLost, msg).retriable(true)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    Graceful(String),
    Transport(String),
    Protocol(String),
    /// The negotiated heartbeat lapsed: no frame of any kind arrived within `timeout_s`, so the
    /// peer is presumed gone even though the socket never errored (half-open).
    Heartbeat(String),
}

impl std::fmt::Display for CloseReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloseReason::Graceful(r) => write!(f, "graceful close: {r}"),
            CloseReason::Transport(r) => write!(f, "transport closed: {r}"),
            CloseReason::Protocol(r) => write!(f, "protocol error: {r}"),
            CloseReason::Heartbeat(r) => write!(f, "heartbeat lapsed: {r}"),
        }
    }
}

/// Why a dial did not produce a connection, in the four shades a dialler acts on differently.
///
/// The split exists because one string could not carry it: `zyris-daemon` kept a `gave_up`
/// `AtomicBool` beside its retry loop precisely because "the server disowned this credential" and
/// "the server answered 500 for a minute" arrived as the same value. The two mistakes are not
/// symmetric — reading an outage as a revocation drags a person back to a terminal, and reading a
/// revocation as an outage is a node that reconnects forever and never comes back.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The grant chain is dead. Nothing a retry can reach will change this answer.
    #[error("this credential was revoked; a person must authorize this node again")]
    Revoked,
    /// The token was judged and refused — a typo, a paste that lost a character, a `zna_` where a
    /// `znt_` belongs.
    #[error("the server did not accept this token")]
    Unauthorized,
    /// Negotiation failed. Retrying gets the same answer; a different build does not.
    #[error(
        "protocol mismatch: this node speaks {ours}, the server speaks {}",
        theirs.as_deref().unwrap_or("unknown")
    )]
    VersionMismatch { ours: String, theirs: Option<String> },
    /// The network or the server, briefly. Retrying is the answer.
    #[error("could not reach the server: {0}")]
    Unreachable(#[from] TransportError),
    /// Rustls picks its provider from crate features, and this build turned none of them on, so
    /// there is nothing to negotiate `wss://` with. Left to rustls this is a panic on the first
    /// connection rather than an error, which is why it is caught here and named.
    #[error(
        "this build selected no TLS provider, so it cannot dial wss://: turn on the `tls-ring` \
         or `tls-aws-lc` feature of this crate, or install one yourself before dialling with \
         `rustls::crypto::CryptoProvider::install_default`"
    )]
    NoTlsProvider,
}

impl From<WireError> for ConnectError {
    fn from(err: WireError) -> ConnectError {
        match err.code {
            ErrorCode::Unauthorized => ConnectError::Unauthorized,
            ErrorCode::UnsupportedVersion => ConnectError::VersionMismatch {
                ours: zyris_proto::PROTOCOL_MAJOR.to_string(),
                theirs: peer_major(&err),
            },
            _ => ConnectError::Unreachable(TransportError::Io(err.message)),
        }
    }
}

impl From<crate::enroll::protocol::RefreshOutcome> for ConnectError {
    fn from(outcome: crate::enroll::protocol::RefreshOutcome) -> ConnectError {
        use crate::enroll::protocol::RefreshOutcome;
        match outcome {
            RefreshOutcome::Dead(_) => ConnectError::Revoked,
            RefreshOutcome::Unavailable(reason) => {
                ConnectError::Unreachable(TransportError::Io(reason))
            }
        }
    }
}

/// The major version the peer named, when it named one.
///
/// The handshake knows exactly which major the peer speaks — it read a `HelloAck` — and puts the
/// number in the error's `data` (`connection.rs`). A 426 at the HTTP upgrade knows nothing: the
/// server refused before either side spoke the protocol. `None` is that second case reported
/// straight, rather than a number invented to fill the field, because a version printed in a log
/// is read as fact.
///
/// It is read out of `data` rather than out of the message because the message is a sentence this
/// crate is free to rewrite, and a typed error a caller has to re-parse is not typed.
fn peer_major(error: &WireError) -> Option<String> {
    error
        .data
        .as_ref()
        .and_then(|data| data.to_json().ok())
        .and_then(|json| json.get("peer_major").and_then(serde_json::Value::as_u64))
        .map(|major| major.to_string())
}

/// Why an enrollment did not produce a credential.
///
/// There is no revoked shade for the enrollment itself: enrolling is what a caller does when it
/// has nothing to present. `Revoked` is here for the refresh that `Account::bearer` performs on a
/// credential it was handed — a grant the server has since disowned, which no retry reaches.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    /// Somebody said no in the browser. Asking again is pestering them.
    #[error("the request was declined")]
    Denied,
    /// The code ran out its clock. Recoverable — ask for another one.
    #[error("the code expired")]
    Lapsed,
    /// The grant chain is dead. A person must authorize this node again.
    #[error("this credential was revoked; a person must authorize this node again")]
    Revoked,
    /// A scope this build asked for does not exist on that deployment. Named, because the server's
    /// own 422 is a serde dump and a caller that cannot read it can only report that *something*
    /// in a list it wrote itself was wrong.
    #[error("the server does not know the scope {scope}")]
    ScopeUnknown { scope: String },
    #[error("could not reach the server: {0}")]
    Unreachable(#[from] TransportError),
}

/// Why an account credential could not mint a node token.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// The account credential was authorized without the one scope this needs.
    #[error("this credential may not register nodes; it lacks nodes:write")]
    Forbidden,
    /// The server clamped the request to what the account grant covers. Both lists travel, so the
    /// difference is a set operation rather than a sentence to parse.
    #[error("requested scopes exceed this credential's grant")]
    ScopeExceeded { requested: Vec<String>, granted: Vec<String> },
    /// The account credential is dead; registering anything under it will keep failing.
    #[error("this credential was revoked; a person must authorize this account again")]
    Revoked,
    #[error("could not reach the server: {0}")]
    Unreachable(#[from] TransportError),
}

/// The caller's own words for why it could not store a rotated credential.
///
/// A string rather than an enum on purpose: this crate has no idea what a keychain, a Kubernetes
/// Secret or a database column is, and inventing shades for them would be guessing.
#[derive(Debug, thiserror::Error)]
#[error("the credential could not be stored: {0}")]
pub struct RotateError(pub String);
