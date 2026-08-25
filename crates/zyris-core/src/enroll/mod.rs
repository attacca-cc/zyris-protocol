//! Self-registration over the OAuth 2.0 Device Authorization Grant (RFC 8628).
//!
//! A node starts with no configuration, is handed a short code, and polls until a person authorizes
//! it from whatever device has a browser. No localhost redirect, and no secret copied between two
//! machines — which is the whole friction on the remote box where a node is most useful.
//!
//! This lives in `zyris` rather than in an example node because every node needs it and a polling
//! loop with `slow_down`/`expired_token` handling is precisely the thing that gets copy-pasted
//! wrong once and stays wrong.
//!
//! `protocol` is always compiled and does no IO of its own, so the retry semantics are readable
//! and testable with no server and no feature flag; `device` sits behind the non-default `enroll`
//! feature so a node using a static `znt_` pays nothing for it.
//!
//! `device` is the only driver: the code comes back as a value, and nothing here writes to a
//! console or decides when to stop polling.
//!
//! **Where the issued credential is kept is not decided here.** It comes back as a value and goes
//! back in as one, so the file, the keychain or the Secret it lives in belongs to the program —
//! which is also what lets one program hold several of them.

pub mod protocol;

#[cfg(feature = "enroll")]
pub mod device;

#[cfg(feature = "enroll")]
pub use device::{enroll, Code, EnrollRequest, Enrollment, Progress};

pub use protocol::{
    classify_refresh_error, AuthorizeRequest, AuthorizeResponse, ClientHint, ErrorResponse,
    PollOutcome, PollState, RefreshOutcome, TokenResponse,
};
