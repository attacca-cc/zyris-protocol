//! The `reqwest` driver for the device grant.
//!
//! Dependency weight is smaller than it looks: `tokio-tungstenite` already pulls in `rustls`,
//! `tokio-rustls`, and the native root store, so `reqwest` configured without default features
//! **reuses that TLS stack** — the marginal cost is the hyper/tower layer. It is off by default, so
//! a node using a static `znt_` pays nothing for it.
//!
//! **Nothing here writes to a console and nothing here loops forever.** The code a person types
//! comes back as a value, and whether that becomes a printed block, a window in a TUI, or a line
//! in a journal is the program's decision, not this crate's.

use std::time::{Duration, Instant, SystemTime};

use crate::account::AccountCredential;
use crate::enroll::protocol::{
    AuthorizeRequest, AuthorizeResponse, ClientHint, ErrorResponse, PollOutcome, PollState,
    TokenResponse,
};
use crate::{EnrollError, TransportError};

/// What a node asks to be enrolled as. The scopes are granted with the credential and never widen
/// afterwards, so this is the one moment they can be chosen.
#[derive(Debug, Clone)]
pub struct EnrollRequest {
    pub name: String,
    pub platform: String,
    pub scopes: Vec<String>,
}

/// What a person has to be shown in order to approve this node.
#[derive(Debug, Clone, PartialEq)]
pub struct Code {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: SystemTime,
}

/// Where one poll left the grant.
#[derive(Debug)]
pub enum Progress {
    /// Nobody has answered yet; the code is good for this much longer.
    Waiting { remaining: Duration },
    Granted(AccountCredential),
    /// The code timed out. Call [`Enrollment::renew`] for a fresh one — this crate never asks for
    /// one on its own, because a loop the caller cannot end is a program, not a library.
    Lapsed,
    Denied,
}

/// One enrollment attempt in progress: the code to show, and the poll that resolves it.
pub struct Enrollment {
    http: reqwest::Client,
    base_url: String,
    request: AuthorizeRequest,
    round: Round,
}

/// One code's worth of state: what to present, what to poll with, and when it stops counting.
struct Round {
    device_code: String,
    code: Code,
    state: PollState,
    /// When the code stops being redeemable, on the monotonic clock so a system clock step cannot
    /// end an attempt early.
    deadline: Instant,
    /// The earliest moment the token endpoint may be asked again.
    next_poll: Instant,
}

impl Round {
    fn begin(authorization: AuthorizeResponse) -> Round {
        let lifetime = Duration::from_secs(authorization.expires_in.max(0) as u64);
        let now = Instant::now();
        Round {
            device_code: authorization.device_code,
            code: Code {
                user_code: authorization.user_code,
                verification_uri: authorization.verification_uri,
                expires_at: SystemTime::now() + lifetime,
            },
            state: PollState::new(authorization.interval),
            deadline: now + lifetime,
            // RFC 8628 bounds the gap *between* polls, so the first one is not made to wait.
            next_poll: now,
        }
    }
}

/// Begin enrolling this node. The returned value carries the code a person has to approve.
///
/// `server_url` is the websocket URL the node dials; the HTTP base is derived from it, so a node
/// is configured with exactly one address and cannot end up enrolling against one deployment while
/// connecting to another.
///
/// The whole flow, which is also the only compile-checked account of it:
///
/// ```no_run
/// use zyris::{Account, AccountCredential, EnrollRequest, Node, NodeSpec, RotateError};
///
/// # async fn flow() -> Result<(), Box<dyn std::error::Error>> {
/// let server = "wss://attacca.cc/api/zyris/v1/ws";
///
/// // The code comes back as a value. Showing it is the caller's decision — a node with a
/// // full-screen UI draws it in a window, and this crate writes to no console either way.
/// let mut enrollment = zyris::enroll(
///     server,
///     EnrollRequest {
///         name: "my node".to_string(),
///         platform: "linux".to_string(),
///         scopes: vec!["nodes:write".to_string()],
///     },
/// )
/// .await?;
/// show(&enrollment.code().user_code, &enrollment.code().verification_uri);
/// let credential: AccountCredential = enrollment.wait().await?;
///
/// // A refresh token is single-use, so a rotation is offered to the caller and adopted only once
/// // the caller says it is stored. A crash between "used" and "saved" is how a node gets revoked.
/// let account = Account::restore(server, credential)
///     .on_rotate(|rotated: AccountCredential| async move {
///         save(&rotated).map_err(|error| RotateError(error.to_string()))
///     })
///     .build();
///
/// // One account credential, as many nodes as you like. The token never expires and never
/// // rotates, so it is this node's identity across restarts — store it, do not mint another.
/// let token = account
///     .register_node(NodeSpec {
///         name: "my node".to_string(),
///         platform: Some("linux".to_string()),
///         scopes: vec![],
///     })
///     .await?;
///
/// // `connect` keeps the link up; `Node::dial` is the single attempt underneath it.
/// let link = Node::builder().name("my node").build()?.connect(server, &token).await?;
/// link.wait_closed().await?;
/// # Ok(())
/// # }
/// # fn show(_code: &str, _uri: &str) {}
/// # fn save(_credential: &zyris::AccountCredential) -> std::io::Result<()> { Ok(()) }
/// ```
pub async fn enroll(server_url: &str, request: EnrollRequest) -> Result<Enrollment, EnrollError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("zyris/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(unreachable_because)?;
    let base_url = http_base(server_url);
    let request = AuthorizeRequest {
        name: request.name,
        platform: request.platform,
        scopes: request.scopes,
        client_hint: local_client_hint(),
    };
    let round = Round::begin(authorize(&http, &base_url, &request).await?);
    Ok(Enrollment { http, base_url, request, round })
}

impl Enrollment {
    /// The code to put in front of a person, and how long it is good for.
    pub fn code(&self) -> &Code {
        &self.round.code
    }

    /// Ask the server once whether the grant has settled, no sooner than the interval it asked for.
    ///
    /// The RFC's semantics — `authorization_pending` is not an error, `slow_down` widens the
    /// interval, `expired_token` means start over rather than give up — are `PollState`'s, and this
    /// only drives it.
    pub async fn poll(&mut self) -> Result<Progress, EnrollError> {
        let wait = self.round.next_poll.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        if Instant::now() >= self.round.deadline {
            // The server would refuse it anyway; saying so without the round trip is the same
            // answer sooner.
            return Ok(Progress::Lapsed);
        }

        let response = post(
            &self.http,
            &self.base_url,
            "/zyris/v1/device/token",
            &serde_json::json!({
                "device_code": self.round.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }),
        )
        .await?;
        self.round.next_poll = Instant::now() + self.round.state.interval();

        if response.status().is_success() {
            let token: TokenResponse = response.json().await.map_err(unreachable_because)?;
            return Ok(Progress::Granted(credential_from(&token)));
        }

        let error = parse_error(response).await;
        match self.round.state.on_error(&error) {
            PollOutcome::KeepWaiting(interval) => {
                self.round.next_poll = Instant::now() + interval;
                Ok(Progress::Waiting {
                    remaining: self.round.deadline.saturating_duration_since(Instant::now()),
                })
            }
            PollOutcome::Expired => Ok(Progress::Lapsed),
            PollOutcome::Denied => Ok(Progress::Denied),
            // A refusal specific enough to name, and not one of the four the caller can act on.
            // `Unreachable` is the only shade that carries the reason, and its retry semantics are
            // the right advice: nothing here can be fixed by asking again immediately.
            PollOutcome::Fatal(message) => {
                Err(EnrollError::Unreachable(TransportError::Io(message)))
            }
        }
    }

    /// Ask for a fresh code, replacing the lapsed one. The caller decides when — a library that
    /// renewed on its own would be a loop nobody asked for.
    pub async fn renew(&mut self) -> Result<(), EnrollError> {
        let authorization = authorize(&self.http, &self.base_url, &self.request).await?;
        self.round = Round::begin(authorization);
        Ok(())
    }

    /// Poll until the grant settles, at the server's cadence.
    ///
    /// A lapsed code ends this rather than renewing itself; call [`renew`](Self::renew) and wait
    /// again if that is what the program wants.
    pub async fn wait(&mut self) -> Result<AccountCredential, EnrollError> {
        loop {
            match self.poll().await? {
                Progress::Waiting { .. } => continue,
                Progress::Granted(credential) => return Ok(credential),
                Progress::Lapsed => return Err(EnrollError::Lapsed),
                Progress::Denied => return Err(EnrollError::Denied),
            }
        }
    }
}

async fn authorize(
    http: &reqwest::Client,
    base_url: &str,
    request: &AuthorizeRequest,
) -> Result<AuthorizeResponse, EnrollError> {
    let response = post(http, base_url, "/zyris/v1/device/authorize", request).await?;
    if response.status().is_success() {
        return response.json().await.map_err(unreachable_because);
    }

    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    // One unknown scope refuses the whole request, before a person ever sees a code — and the name
    // is the only thing that tells the caller which one to drop.
    if let Some(scope) = unknown_scope(&body) {
        return Err(EnrollError::ScopeUnknown { scope });
    }
    Err(EnrollError::Unreachable(TransportError::Io(describe(&error_of(status, &body)))))
}

/// `{"error":"unknown_scope","scope":"nodes:write"}`. Anything else — a different refusal, an
/// ingress's HTML — is not this, and must not be reported as if the caller could act on it.
fn unknown_scope(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("error")?.as_str()? != "unknown_scope" {
        return None;
    }
    Some(value.get("scope")?.as_str()?.to_string())
}

pub(crate) fn credential_from(token: &TokenResponse) -> AccountCredential {
    AccountCredential::new(
        token.access_token.clone(),
        token.refresh_token.clone(),
        token.node_id.clone(),
        token.node_name.clone(),
        token.owner_email.clone(),
        now_unix() + token.expires_in,
    )
}

pub(crate) async fn post<T: serde::Serialize + ?Sized>(
    http: &reqwest::Client,
    base_url: &str,
    path: &str,
    body: &T,
) -> Result<reqwest::Response, EnrollError> {
    http.post(format!("{base_url}{path}")).json(body).send().await.map_err(unreachable_because)
}

pub(crate) fn unreachable_because(error: reqwest::Error) -> EnrollError {
    EnrollError::Unreachable(TransportError::Io(error.to_string()))
}

pub(crate) async fn parse_error(response: reqwest::Response) -> ErrorResponse {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    error_of(status, &body)
}

/// The status is stamped on both paths, not just the synthesised one: a proxy can return 503 with
/// a perfectly well-formed error body, and that is still the transport failing rather than the
/// grant being refused. `#[serde(skip)]` means a parsed value arrives here as `None`.
fn error_of(status: u16, body: &str) -> ErrorResponse {
    let mut error = serde_json::from_str::<ErrorResponse>(body).unwrap_or(ErrorResponse {
        error: format!("http_{status}"),
        error_description: None,
        interval: None,
        status: None,
    });
    error.status = Some(status);
    error
}

fn describe(error: &ErrorResponse) -> String {
    match &error.error_description {
        Some(description) => format!("{}: {description}", error.error),
        None => error.error.clone(),
    }
}

/// `ws://…/zyris/v1/ws` → `http://…`. A node is configured with one address, so the enrollment
/// endpoints are derived from it rather than being a second thing to get wrong.
pub(crate) fn http_base(server_url: &str) -> String {
    let without_scheme = server_url
        .strip_prefix("wss://")
        .map(|rest| format!("https://{rest}"))
        .or_else(|| server_url.strip_prefix("ws://").map(|rest| format!("http://{rest}")))
        .unwrap_or_else(|| server_url.to_string());
    match without_scheme.find("/zyris/") {
        Some(index) => without_scheme[..index].to_string(),
        None => without_scheme.trim_end_matches('/').to_string(),
    }
}

/// What this machine says about itself. Every field is a hint the server never verifies, and the
/// approval screen labels it as such.
fn local_client_hint() -> ClientHint {
    ClientHint {
        hostname: hostname(),
        os: Some(std::env::consts::OS.to_string()),
        agent: Some(concat!("zyris/", env!("CARGO_PKG_VERSION")).to_string()),
    }
}

#[cfg(feature = "hostname")]
fn hostname() -> Option<String> {
    crate::machine_name()
}

#[cfg(not(feature = "hostname"))]
fn hostname() -> Option<String> {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|v| v.trim().to_string()))
        .filter(|v| !v.is_empty())
}

pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// zyris:nothing below this ships. Everything past this line is `#[cfg(test)]`, and the console
// scan in `mod tests` bounds itself here rather than at the first `#[cfg(test)]` it finds, which
// would move the moment another test helper landed above shipping code. New shipping code goes
// above this line.

/// Answer on a fresh loopback port, picking by path — and, when a path is scripted more than once,
/// giving a different answer per request with the last one repeating.
///
/// A real socket rather than a faked HTTP client, because the branches under test key off a status
/// line and a response body, and stubbing the client would stub out exactly the input that decides
/// them. Enrollment is a conversation: the token endpoint has to be able to say something different
/// the second time it is asked.
#[cfg(test)]
pub(crate) fn scripted_responder(
    scripts: &'static [(&'static str, &'static str, &'static str)],
) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
    let address = listener.local_addr().expect("no local address");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut used: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buf = [0u8; 4096];
            let Ok(n) = stream.read(&mut buf) else { continue };
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();

            let matching: Vec<&(&str, &str, &str)> =
                scripts.iter().filter(|(suffix, _, _)| path.contains(suffix)).collect();
            let (status, body) = if matching.is_empty() {
                // Answering rather than hanging: an unscripted path should fail a test in seconds,
                // not at the client's 30-second timeout.
                ("404 Not Found", r#"{"error":"nothing scripted for this path"}"#)
            } else {
                let seen = used.entry(path).or_insert(0);
                let (_, status, body) = *matching[(*seen).min(matching.len() - 1)];
                *seen += 1;
                (status, body)
            };

            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("ws://{address}/zyris/v1/ws")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_CODE: &str = r#"{"device_code":"zdc_secret","user_code":"WXQR-7KBD","verification_uri":"https://attacca.example/settings/zyris/device","expires_in":600,"interval":1}"#;
    const SECOND_CODE: &str = r#"{"device_code":"zdc_second","user_code":"HTPL-2FMR","verification_uri":"https://attacca.example/settings/zyris/device","expires_in":600,"interval":1}"#;
    const GRANTED: &str = r#"{"access_token":"zna_new","refresh_token":"znr_new","expires_in":3600,"scope":"agents:read","node_id":"node-7","node_name":"hello node","owner_email":"allen@example.com"}"#;
    const PENDING: &str = r#"{"error":"authorization_pending"}"#;

    fn request() -> EnrollRequest {
        EnrollRequest {
            name: "hello node".into(),
            platform: "linux".into(),
            scopes: vec!["agents:read".into()],
        }
    }

    /// The code comes back as a value and this module says nothing anywhere else. A `println!` in
    /// here lands underneath a TUI's own screen, or in a journal nobody reads — three consumers of
    /// this crate each had to work around exactly that.
    ///
    /// The name says *this module* because that is all the scan below reads. The same rule across
    /// the whole crate is `tests/library_shape.rs`, and it runs on every `cargo test`: the printing
    /// enroll driver that once forced it to be `#[ignore]`d is gone, and `enroll/http.rs` with it.
    /// So a green run here is no longer the only evidence — this is the narrow scan, bounded to the
    /// shipping half of one file, and it fails on the same run that checks the code is still handed
    /// back as a value rather than written somewhere.
    #[tokio::test]
    async fn the_code_comes_back_as_a_value_and_this_module_writes_to_no_console() {
        let url = scripted_responder(&[("/device/authorize", "200 OK", FIRST_CODE)]);

        let enrollment = enroll(&url, request()).await.expect("the server answered");

        assert_eq!(enrollment.code().user_code, "WXQR-7KBD");
        assert_eq!(
            enrollment.code().verification_uri,
            "https://attacca.example/settings/zyris/device"
        );
        assert!(
            enrollment.code().expires_at > SystemTime::now(),
            "a code that is already expired when it is handed over is not a code"
        );

        // The needles are spelled in halves so they are not themselves in the file being
        // searched, and only what ships is searched: the doc comment above has to be able to
        // *name* a console macro in order to say why there is none, and that halving trick does
        // not reach into a doc comment.
        //
        // The boundary is an explicit marker rather than the first `#[cfg(test)]`. That one guards
        // a test helper partway up the file, so a scan bounded by it stops reading while shipping
        // code is still to come, and every console write below the cut goes unseen while this
        // stays green. A marker moves only when somebody moves it. It is spelled in halves too, so
        // the one literal copy in this file is the marker itself.
        let marker = concat!("// zyris:", "nothing below this ships");
        let mut parts = include_str!("device.rs").split(marker);
        let shipped = parts.next().expect("splitting yields a first part even with no match");
        assert!(
            parts.next().is_some(),
            "the marker bounding the shipping code is gone, so this scan read the whole file as \
             test-only and asserted over nothing"
        );
        assert!(
            parts.next().is_none(),
            "the boundary is named twice, so the scan stops at whichever comes first"
        );
        assert!(
            shipped.contains("pub async fn enroll"),
            "the scan ended before this module's own entry point; the boundary has drifted upwards"
        );
        for console in [concat!("print", "ln!"), concat!("eprint", "ln!"), concat!("print", "!(")] {
            assert!(!shipped.contains(console), "{console} writes where the caller cannot see it");
        }
    }

    /// The ordinary answer to almost every poll. `remaining` is what a caller draws a countdown
    /// with, so it has to describe the code rather than the request that just came back.
    #[tokio::test]
    async fn a_grant_nobody_has_answered_yet_says_how_long_the_code_has_left() {
        let url = scripted_responder(&[
            ("/device/authorize", "200 OK", FIRST_CODE),
            ("/device/token", "400 Bad Request", PENDING),
        ]);
        let mut enrollment = enroll(&url, request()).await.expect("the server answered");

        match enrollment.poll().await.expect("pending is not a failure") {
            Progress::Waiting { remaining } => assert!(
                remaining > Duration::from_secs(500) && remaining <= Duration::from_secs(600),
                "the countdown must be the code's, got {remaining:?}"
            ),
            other => panic!("expected to still be waiting, got {other:?}"),
        }
    }

    /// `wait` is the convenience the documented loop collapses into: poll at the server's cadence
    /// until the grant settles. What comes out is the credential itself — the caller stores it.
    #[tokio::test]
    async fn waiting_out_a_pending_grant_ends_with_the_account_credential() {
        let url = scripted_responder(&[
            ("/device/authorize", "200 OK", FIRST_CODE),
            ("/device/token", "400 Bad Request", PENDING),
            ("/device/token", "200 OK", GRANTED),
        ]);
        let mut enrollment = enroll(&url, request()).await.expect("the server answered");

        let credential = enrollment.wait().await.expect("the person approved it");

        assert_eq!(credential.access_token, "zna_new");
        assert_eq!(credential.refresh_token, "znr_new");
        assert_eq!(credential.node_id, "node-7");
        assert_eq!(credential.owner_email, "allen@example.com");
        assert!(
            credential.bearer(now_unix(), 30).is_some(),
            "a credential handed over already spent is not one"
        );
    }

    /// A lapsed code is reported, not replaced. A library that fetches another one on its own has
    /// a loop the caller cannot end — which is the thing this whole redesign is removing.
    #[tokio::test]
    async fn a_lapsed_code_is_reported_and_no_fresh_one_is_fetched_behind_the_caller() {
        let url = scripted_responder(&[
            ("/device/authorize", "200 OK", FIRST_CODE),
            ("/device/authorize", "200 OK", SECOND_CODE),
            ("/device/token", "400 Bad Request", r#"{"error":"expired_token"}"#),
        ]);
        let mut enrollment = enroll(&url, request()).await.expect("the server answered");

        assert!(
            matches!(enrollment.poll().await.expect("lapsing is not a failure"), Progress::Lapsed),
            "the caller has to be told the code stopped working"
        );
        assert_eq!(
            enrollment.code().user_code,
            "WXQR-7KBD",
            "renewing is the caller's call; a second code must not appear on its own"
        );
    }

    /// And when the caller does ask, it gets a genuinely new code — the lapsed one would be
    /// refused for as long as it were shown.
    #[tokio::test]
    async fn renewing_asks_for_a_new_code_rather_than_showing_the_lapsed_one_again() {
        let url = scripted_responder(&[
            ("/device/authorize", "200 OK", FIRST_CODE),
            ("/device/authorize", "200 OK", SECOND_CODE),
        ]);
        let mut enrollment = enroll(&url, request()).await.expect("the server answered");
        assert_eq!(enrollment.code().user_code, "WXQR-7KBD");

        enrollment.renew().await.expect("the server issued another");

        assert_eq!(enrollment.code().user_code, "HTPL-2FMR");
        assert!(enrollment.code().expires_at > SystemTime::now(), "and it is good for a while");
    }

    /// Declined is an outcome, not a transport failure. A caller that reads it as one would pester
    /// someone who already said no.
    #[tokio::test]
    async fn a_declined_request_is_reported_as_declined() {
        let url = scripted_responder(&[
            ("/device/authorize", "200 OK", FIRST_CODE),
            (
                "/device/token",
                "400 Bad Request",
                r#"{"error":"access_denied","error_description":"the user declined"}"#,
            ),
        ]);
        let mut enrollment = enroll(&url, request()).await.expect("the server answered");

        assert!(matches!(
            enrollment.poll().await.expect("a refusal is an answer"),
            Progress::Denied
        ));
    }

    /// A node is configured with one address; enrolling against a different deployment than it
    /// connects to is exactly the mistake this derivation exists to make impossible.
    #[test]
    fn http_base_is_derived_from_the_websocket_url() {
        assert_eq!(http_base("wss://attacca.example/zyris/v1/ws"), "https://attacca.example");
        assert_eq!(http_base("ws://127.0.0.1:8080/zyris/v1/ws"), "http://127.0.0.1:8080");
        assert_eq!(http_base("https://attacca.example/"), "https://attacca.example");
        assert_eq!(
            http_base("wss://attacca.example:8443/zyris/v1/ws"),
            "https://attacca.example:8443"
        );
        // The default deployment sits behind an ingress that strips `/api`, so the prefix is part
        // of the address a node is given and has to survive into the enrollment endpoints.
        assert_eq!(
            http_base(crate::DEFAULT_SERVER_URL),
            "https://attacca.cc/api",
            "the enrollment base must keep the path prefix the websocket URL carries"
        );
    }

    #[test]
    fn the_client_hint_describes_this_machine() {
        let hint = local_client_hint();
        assert_eq!(hint.os.as_deref(), Some(std::env::consts::OS));
        assert!(hint.agent.as_deref().unwrap().starts_with("zyris/"));
    }

    #[test]
    fn an_unparseable_error_body_still_names_the_status() {
        assert_eq!(error_of(503, "<html>bad gateway</html>").error, "http_503");
        assert_eq!(error_of(503, "<html>bad gateway</html>").status, Some(503));
    }

    /// Enrol and expect to be turned away. `Enrollment` is not `Debug`, so `expect_err` cannot
    /// report the other side; naming the code that came back instead says more than a bare unwrap
    /// would, and no shipping type has to grow a trait for a test's sake.
    async fn refusal_from(url: &str) -> EnrollError {
        match enroll(url, request()).await {
            Err(refusal) => refusal,
            Ok(enrollment) => {
                panic!("expected a refusal, got the code {}", enrollment.code().user_code)
            }
        }
    }

    /// One unknown scope refuses the whole request before a person ever sees a code, and the name
    /// is the only thing a caller can act on — drop that scope and ask again.
    ///
    /// Driven through `enroll` rather than through the parser alone, because reading a name out of
    /// a body proves nothing about whether the refusal ever reaches the caller: with the branch in
    /// `authorize` taken out, a test that stopped at the parser stayed green.
    #[tokio::test]
    async fn a_scope_the_server_has_never_heard_of_comes_back_by_name() {
        let url = scripted_responder(&[(
            "/device/authorize",
            "422 Unprocessable Entity",
            r#"{"error":"unknown_scope","scope":"nodes:write"}"#,
        )]);

        let refusal = refusal_from(&url).await;

        match refusal {
            EnrollError::ScopeUnknown { scope } => assert_eq!(scope, "nodes:write"),
            other => panic!("a caller cannot drop a scope it was not handed the name of: {other}"),
        }
    }

    /// Any other refusal is not that one. Dressed up as `ScopeUnknown` it would send a caller off
    /// dropping a scope that was never the problem, and the request would be refused again.
    #[tokio::test]
    async fn a_refusal_that_names_no_scope_is_not_reported_as_an_unknown_one() {
        let url = scripted_responder(&[(
            "/device/authorize",
            "422 Unprocessable Entity",
            r#"{"error":"invalid_request"}"#,
        )]);

        let refusal = refusal_from(&url).await;

        assert!(
            matches!(refusal, EnrollError::Unreachable(_)),
            "only a refusal that names a scope may arrive as one, got {refusal}"
        );
        // The parser underneath draws the same line, including for a body no status code explains
        // — an ingress answering with HTML.
        assert_eq!(unknown_scope(r#"{"error":"invalid_request"}"#), None);
        assert_eq!(unknown_scope("<html>422</html>"), None);
    }
}
