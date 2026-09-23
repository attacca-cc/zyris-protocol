//! Wire types and the polling state machine for the OAuth 2.0 Device Authorization Grant
//! (RFC 8628), as a node speaks it.
//!
//! **This module is always compiled and performs no IO.** That is deliberate: the retry semantics
//! below — `slow_down` widening the interval, `authorization_pending` not being an error,
//! `expired_token` meaning "start over" rather than "give up" — are exactly the logic that gets
//! copy-pasted wrong once and stays wrong. Keeping them pure means they can be unit-tested with no
//! server, no network, and no feature flag, which is the only way anyone will actually verify them.
//! It also lets a node with its own HTTP client use these types without taking `reqwest`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::credential::Named;

/// Lower bound on any server-supplied interval. A server that answered `0` would otherwise turn a
/// polite poller into a hot loop.
pub const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Upper bound, so a bad or hostile value cannot park a node past its own code's lifetime.
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Applied when the server reports `slow_down` without naming a new interval.
pub const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct AuthorizeRequest {
    /// The program asking, as it names itself: `zyris-code`. Fixed on the credential.
    pub program: String,
    /// What this machine calls itself. The approval screen preselects the system of that name.
    pub system_hint: String,
    pub platform: String,
    pub scopes: Vec<String>,
    pub client_hint: ClientHint,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    /// `zc_…`.
    pub credential: String,
    pub system: Named,
    pub program: Named,
    pub scopes: Vec<String>,
    /// The account the credential was issued under. Printed on success because, over SSH against a
    /// URL typed from memory, this line is the only chance to notice you enrolled into the wrong
    /// deployment.
    #[serde(default)]
    pub owner_email: String,
}

/// The RFC 6749 §5.2 error body. `interval` is Attacca's addition, so a node can adopt the
/// server's widened interval instead of guessing.
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(default)]
    pub error_description: Option<String>,
    #[serde(default)]
    pub interval: Option<i32>,
    /// The HTTP status the body arrived with. **Not part of the wire format** — the transport
    /// stamps it on, because the body alone cannot distinguish the two things that matter here:
    /// the server answering "no" and the server not answering at all. An edge returning 502 with
    /// an HTML page is read as `http_502`, which looks exactly like an unrecognised error code.
    #[serde(skip)]
    pub status: Option<u16>,
}

impl ErrorResponse {
    /// Whether the status says the server could not answer, rather than that it answered.
    ///
    /// 429 is in here with the 5xx family because it means the same thing to a caller deciding
    /// whether to ask again: the request was never judged on its merits.
    pub fn is_transient(&self) -> bool {
        matches!(self.status, Some(status) if status == 429 || (500..600).contains(&status))
    }
}

/// What one poll of the token endpoint means to the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// Keep waiting; sleep for the enclosed interval first.
    KeepWaiting(Duration),
    /// The code lapsed. Request a new one and print a fresh block — not a fatal condition.
    Expired,
    /// The user said no. Fatal: retrying would be pestering someone who already declined.
    Denied,
    /// Unrecoverable and specific enough to name.
    Fatal(String),
}

/// Tracks a single enrollment attempt's polling cadence. Holding this as a value rather than
/// scattering the arithmetic across the request loop is what makes the whole thing testable.
#[derive(Debug, Clone)]
pub struct PollState {
    interval: Duration,
}

impl PollState {
    /// Start from the server's advertised interval, clamped into a sane range.
    pub fn new(interval_secs: i32) -> PollState {
        PollState { interval: clamp_interval(Duration::from_secs(interval_secs.max(0) as u64)) }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Fold one error response into the next action.
    ///
    /// `authorization_pending` is the *expected* answer for almost every poll, and `slow_down` is
    /// a correction rather than a failure — treating either as fatal is the classic way to write
    /// a device-grant client that never completes.
    pub fn on_error(&mut self, response: &ErrorResponse) -> PollOutcome {
        match response.error.as_str() {
            "authorization_pending" => {
                if let Some(interval) = response.interval {
                    self.adopt(interval);
                }
                PollOutcome::KeepWaiting(self.interval)
            }
            "slow_down" => {
                // Prefer the server's number; it knows what it actually enforced. Absent one, back
                // off by the RFC's suggested step rather than keeping the same cadence.
                match response.interval {
                    Some(interval) => self.adopt(interval),
                    None => self.interval = clamp_interval(self.interval + SLOW_DOWN_STEP),
                }
                PollOutcome::KeepWaiting(self.interval)
            }
            "expired_token" => PollOutcome::Expired,
            "access_denied" => PollOutcome::Denied,
            // A 502 from an edge in front of Attacca is not an answer about this grant. The code
            // is still unexpired and the person may not even have reached the screen yet, so
            // giving up throws away a live code and makes them start over — which is what a
            // single 502 mid-poll actually did. Back off and ask again; `expires_in` already
            // bounds how long that can go on.
            _ if response.is_transient() => {
                self.interval = clamp_interval(self.interval + SLOW_DOWN_STEP);
                PollOutcome::KeepWaiting(self.interval)
            }
            other => PollOutcome::Fatal(match &response.error_description {
                Some(description) => format!("{other}: {description}"),
                None => other.to_string(),
            }),
        }
    }

    fn adopt(&mut self, interval_secs: i32) {
        // Only ever widen from a server hint. A server that answered with a *smaller* interval
        // than the node already backed off to would undo a `slow_down` it just issued.
        let proposed = clamp_interval(Duration::from_secs(interval_secs.max(0) as u64));
        self.interval = self.interval.max(proposed);
    }
}

fn clamp_interval(interval: Duration) -> Duration {
    interval.clamp(MIN_POLL_INTERVAL, MAX_POLL_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: &str, interval: Option<i32>) -> ErrorResponse {
        ErrorResponse {
            error: kind.to_string(),
            error_description: Some("because".to_string()),
            interval,
            status: None,
        }
    }

    /// The path a real enrollment takes: pending for a while, one correction, then success. If
    /// this ever reads as anything but "keep waiting", nodes stop enrolling.
    #[test]
    fn pending_then_slow_down_then_still_pending() {
        let mut state = PollState::new(5);
        assert_eq!(state.interval(), Duration::from_secs(5));

        assert_eq!(
            state.on_error(&error("authorization_pending", None)),
            PollOutcome::KeepWaiting(Duration::from_secs(5))
        );

        // The server widened its own interval and said so; the node adopts it verbatim.
        assert_eq!(
            state.on_error(&error("slow_down", Some(10))),
            PollOutcome::KeepWaiting(Duration::from_secs(10))
        );
        assert_eq!(
            state.on_error(&error("authorization_pending", Some(10))),
            PollOutcome::KeepWaiting(Duration::from_secs(10))
        );
    }

    /// A `slow_down` with no interval must still back off, or the node keeps making the mistake
    /// the server just complained about.
    #[test]
    fn slow_down_without_a_hint_still_widens() {
        let mut state = PollState::new(5);
        assert_eq!(
            state.on_error(&error("slow_down", None)),
            PollOutcome::KeepWaiting(Duration::from_secs(10))
        );
        assert_eq!(
            state.on_error(&error("slow_down", None)),
            PollOutcome::KeepWaiting(Duration::from_secs(15))
        );
    }

    /// A server hint must never *shrink* the interval — that would let a reply undo the very
    /// `slow_down` that preceded it.
    #[test]
    fn a_smaller_server_hint_never_undoes_a_backoff() {
        let mut state = PollState::new(5);
        state.on_error(&error("slow_down", Some(30)));
        assert_eq!(state.interval(), Duration::from_secs(30));
        state.on_error(&error("authorization_pending", Some(5)));
        assert_eq!(state.interval(), Duration::from_secs(30));
    }

    #[test]
    fn intervals_are_clamped_at_both_ends() {
        assert_eq!(PollState::new(0).interval(), MIN_POLL_INTERVAL, "0 must not become a hot loop");
        assert_eq!(PollState::new(-5).interval(), MIN_POLL_INTERVAL);
        assert_eq!(PollState::new(9999).interval(), MAX_POLL_INTERVAL);

        let mut state = PollState::new(5);
        for _ in 0..100 {
            state.on_error(&error("slow_down", None));
        }
        assert_eq!(state.interval(), MAX_POLL_INTERVAL);
    }

    /// Expiry is recoverable (ask for a new code) and denial is not (someone said no). Confusing
    /// the two either strands a node forever or pesters a user who already declined.
    #[test]
    fn expiry_and_denial_are_distinct() {
        let mut state = PollState::new(5);
        assert_eq!(state.on_error(&error("expired_token", None)), PollOutcome::Expired);
        assert_eq!(state.on_error(&error("access_denied", None)), PollOutcome::Denied);
    }

    #[test]
    fn unknown_errors_are_fatal_and_keep_the_description() {
        let mut state = PollState::new(5);
        let outcome = state.on_error(&error("invalid_grant", None));
        let PollOutcome::Fatal(message) = outcome else { panic!("expected fatal") };
        assert!(message.contains("invalid_grant"));
        assert!(message.contains("because"), "the server's reason must survive");
    }

    /// Observed live: a Windows node polling with ~5 minutes left on its code took one 502 from
    /// the edge and printed "the server rejected this node", ending the attempt. The code was
    /// still good and the person had not yet approved it.
    #[test]
    fn a_transient_5xx_keeps_waiting_instead_of_ending_the_attempt() {
        for status in [429u16, 500, 502, 503, 504] {
            let mut state = PollState::new(5);
            let response = ErrorResponse {
                error: format!("http_{status}"),
                error_description: None,
                interval: None,
                status: Some(status),
            };
            let PollOutcome::KeepWaiting(interval) = state.on_error(&response) else {
                panic!("{status} ended the enrollment; it says nothing about the grant")
            };
            assert_eq!(
                interval,
                Duration::from_secs(10),
                "an unreachable server should be asked less often, not given up on"
            );
        }
    }

    /// The other edge of the same rule. Retrying must not swallow answers the server actually
    /// gave — a 400 naming an error code this client does not know is a real refusal, and polling
    /// on until the code expires would hide it behind a timeout.
    #[test]
    fn a_four_hundred_with_an_unknown_code_is_still_fatal() {
        let mut state = PollState::new(5);
        let response = ErrorResponse {
            error: "unsupported_grant_type".into(),
            error_description: Some("because".into()),
            interval: None,
            status: Some(400),
        };
        let PollOutcome::Fatal(message) = state.on_error(&response) else {
            panic!("a 400 is the server answering, and this client cannot honour that answer")
        };
        assert!(message.contains("unsupported_grant_type"));
    }

    /// The two bodies Attacca reads and writes, spelled out. A renamed field here is an enrollment
    /// that passes every stub and fails against the live server.
    #[test]
    fn the_device_grant_bodies_are_the_ones_the_server_speaks() {
        let request = AuthorizeRequest {
            program: "zyris-code".into(),
            system_hint: "laptop".into(),
            platform: "linux".into(),
            scopes: vec!["agents:read".into()],
            client_hint: ClientHint {
                hostname: Some("laptop".into()),
                os: Some("linux".into()),
                agent: Some("zyris/0.3.0".into()),
            },
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "program": "zyris-code", "system_hint": "laptop", "platform": "linux",
                "scopes": ["agents:read"],
                "client_hint": {"hostname": "laptop", "os": "linux", "agent": "zyris/0.3.0"}
            })
        );

        let token: TokenResponse = serde_json::from_str(
            r#"{"credential":"zc_new",
                "system":{"id":"sys-1","name":"laptop","slug":"laptop"},
                "program":{"id":"cred-1","name":"zyris-code","slug":"zyris-code"},
                "scopes":["agents:read"],"owner_email":"allen@example.com"}"#,
        )
        .unwrap();
        assert_eq!(token.credential, "zc_new");
        assert_eq!(token.system.slug, "laptop");
        assert_eq!(token.program.id, "cred-1");
        assert_eq!(token.scopes, vec!["agents:read".to_string()]);
    }
}
