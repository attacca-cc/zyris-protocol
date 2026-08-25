//! The account credential: what a device grant issues, and what the caller stores.
//!
//! One account credential can stand up any number of nodes. It is the only thing in this crate
//! that rotates — a `znt_` node token is static, which is why nothing here has to lock a file.
//!
//! Where it lives between runs is the caller's business. This crate never picks a path: a laptop
//! wants a file under `$HOME`, a pod wants a Secret, a desktop app wants the OS keychain, and a
//! test wants nothing at all.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::enroll::device::{
    credential_from, http_base, now_unix, parse_error, post, unreachable_because,
};
use crate::enroll::protocol::{classify_refresh_error, RefreshOutcome, TokenResponse};
use crate::{EnrollError, RegisterError, RotateError, TransportError};

/// Bumped only on an incompatible change. A credential from the future is refused rather than
/// guessed at, because guessing wrong here means a node that authenticates as something unintended.
const CREDENTIAL_VERSION: u32 = 1;

/// An account's long-lived identity: the refresh token it re-presents forever, and the access
/// token it presents at each dial.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountCredential {
    pub version: u32,
    pub access_token: String,
    pub refresh_token: String,
    pub node_id: String,
    #[serde(default)]
    pub node_name: String,
    #[serde(default)]
    pub owner_email: String,
    /// Unix seconds. Stored as an absolute instant rather than the `expires_in` the server sent,
    /// because the value has to survive the process that received it.
    pub access_expires_at: i64,
}

impl AccountCredential {
    pub fn new(
        access_token: String,
        refresh_token: String,
        node_id: String,
        node_name: String,
        owner_email: String,
        access_expires_at: i64,
    ) -> AccountCredential {
        AccountCredential {
            version: CREDENTIAL_VERSION,
            access_token,
            refresh_token,
            node_id,
            node_name,
            owner_email,
            access_expires_at,
        }
    }

    /// The bearer to present at the websocket upgrade, or `None` when the access token is spent.
    ///
    /// `skew_secs` is subtracted so a token that would expire during the handshake is refreshed
    /// instead of being raced.
    pub fn bearer(&self, now_unix: i64, skew_secs: i64) -> Option<&str> {
        (self.access_expires_at - skew_secs > now_unix).then_some(self.access_token.as_str())
    }

    /// Whether it is worth refreshing ahead of time. At 80% of a one-hour lifetime this fires with
    /// twelve minutes to spare, which is enough to absorb a transient failure and retry.
    pub fn should_refresh(&self, now_unix: i64, lifetime_secs: i64) -> bool {
        let refresh_at = self.access_expires_at - (lifetime_secs / 5);
        now_unix >= refresh_at
    }
}

/// Clock-skew allowance when deciding whether the held access token is still worth presenting.
const SKEW_SECS: i64 = 30;
/// The access-token lifetime the server issues. Used only to decide when to rotate early.
const ACCESS_LIFETIME_SECS: i64 = 3600;

type Rotation = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), RotateError>> + Send>>;
type RotateHook = Arc<dyn Fn(AccountCredential) -> Rotation + Send + Sync>;

/// A live account credential: the thing that rotates, and the thing node tokens are minted against.
pub struct Account {
    http: reqwest::Client,
    base_url: String,
    held: tokio::sync::Mutex<AccountCredential>,
    on_rotate: RotateHook,
}

pub struct AccountBuilder {
    server_url: String,
    credential: AccountCredential,
    on_rotate: Option<RotateHook>,
}

impl Account {
    /// Pick up a credential the caller stored. `server_url` is the websocket URL this account was
    /// enrolled against; the HTTP base is derived from it, as everywhere else in this crate.
    pub fn restore(server_url: &str, credential: AccountCredential) -> AccountBuilder {
        AccountBuilder { server_url: server_url.to_string(), credential, on_rotate: None }
    }

    /// The access token to present, rotating first if the one held is due.
    ///
    /// **A rotation reaches `on_rotate` before it is adopted, and a hook that fails leaves the old
    /// credential in use.** The refresh token is single-use: if this process began presenting a
    /// pair that never reached the caller's storage, the next start would present the spent one,
    /// and attacca reads a replay past its 30-second grace as a leaked chain — that is
    /// `revoke_all_for_node`, which kills every node under this credential, not just this one.
    ///
    /// Rotating at 80% of the lifetime rather than at expiry is what makes that safe to enforce:
    /// there are roughly twelve minutes of slack in which a hook can fail without stopping a dial.
    pub async fn bearer(&self) -> Result<String, EnrollError> {
        let mut held = self.held.lock().await;
        if held.should_refresh(now_unix(), ACCESS_LIFETIME_SECS) {
            match self.rotate(&held).await {
                Ok(rotated) => *held = rotated,
                Err(error) => {
                    // Only fatal when there is nothing left to hand back. A hook or a server having
                    // a bad moment must not stop a node that is holding a perfectly good token.
                    if held.bearer(now_unix(), SKEW_SECS).is_none() {
                        return Err(error);
                    }
                    tracing::warn!(%error, "could not rotate; carrying on with the credential held");
                }
            }
        }
        held.bearer(now_unix(), SKEW_SECS).map(str::to_string).ok_or_else(|| {
            EnrollError::Unreachable(TransportError::Io(
                "the credential just rotated is already expired; check this machine's clock"
                    .to_string(),
            ))
        })
    }

    /// The credential as it stands now — what the caller would store if it were asked again.
    pub async fn credential(&self) -> AccountCredential {
        self.held.lock().await.clone()
    }

    /// Refresh, then hand the result to the caller's hook. Nothing here writes to `self.held`;
    /// `bearer` adopts only what came back through the hook.
    async fn rotate(&self, held: &AccountCredential) -> Result<AccountCredential, EnrollError> {
        let rotated = self.refresh(&held.refresh_token).await?;
        (self.on_rotate)(rotated.clone()).await.map_err(|error| {
            // `EnrollError` has no shade for "the caller could not keep it". `Unreachable` is the
            // one that carries the reason, and its advice — try again later — is the right one.
            EnrollError::Unreachable(TransportError::Io(error.to_string()))
        })?;
        Ok(rotated)
    }

    async fn refresh(&self, refresh_token: &str) -> Result<AccountCredential, EnrollError> {
        let response = post(
            &self.http,
            &self.base_url,
            "/zyris/v1/device/refresh",
            &serde_json::json!({ "refresh_token": refresh_token }),
        )
        .await?;
        if !response.status().is_success() {
            // Classified rather than assumed fatal: a 503 during a deploy would otherwise read as
            // "this account is over" for every node that rotated through it.
            let error = parse_error(response).await;
            return Err(match classify_refresh_error(&error) {
                // The chain is gone and no retry reaches a different answer; a person has to
                // authorize this account again, which is what `Revoked` tells the caller.
                RefreshOutcome::Dead(reason) => {
                    tracing::warn!(%reason, "this account credential will not be honoured again");
                    EnrollError::Revoked
                }
                RefreshOutcome::Unavailable(reason) => {
                    EnrollError::Unreachable(TransportError::Io(reason))
                }
            });
        }
        let token: TokenResponse = response.json().await.map_err(unreachable_because)?;
        Ok(credential_from(&token))
    }
}

impl AccountBuilder {
    /// Where a rotated credential goes. It is adopted only once this returns `Ok` — see
    /// [`Account::bearer`] for why that ordering is the difference between a node that keeps
    /// running and an account that gets revoked.
    ///
    /// With no hook set, a rotation is used and then lost when the process ends. That is a choice
    /// for a short-lived worker and a mistake for anything else.
    pub fn on_rotate<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(AccountCredential) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<(), RotateError>> + Send + 'static,
    {
        self.on_rotate = Some(Arc::new(move |credential| Box::pin(f(credential))));
        self
    }

    pub fn build(self) -> Account {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("zyris/", env!("CARGO_PKG_VERSION")))
            .build()
            // A client that will not build means there is no TLS backend at all, which neither the
            // timeout nor the user agent caused. Falling back keeps `build` infallible, as its
            // signature promises.
            .unwrap_or_default();
        Account {
            http,
            base_url: http_base(&self.server_url),
            held: tokio::sync::Mutex::new(self.credential),
            on_rotate: self.on_rotate.unwrap_or_else(|| Arc::new(|_| Box::pin(async { Ok(()) }))),
        }
    }
}

/// What a node is asked to be registered as. `scopes` is a request, not a grant: the server clamps
/// it to what this account credential was given.
///
/// There is no kind here on purpose. Attacca stores no node kind — it reads one off the `Hello`
/// frame at connect time, so a kind in the registration body would be validated and discarded. A
/// node declares what it is when it connects, through `NodeBuilder::kind`.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub name: String,
    pub platform: Option<String>,
    pub scopes: Vec<String>,
}

/// A minted node token and the identity it carries. `znt_` tokens do not expire and are never
/// rotated, so nothing here has to be handed back to the caller's storage on a schedule — but it
/// is the only copy, and this crate does not keep one.
///
/// It is serializable for the same reason [`AccountCredential`] is: the caller is the only place it
/// can live, and a value a caller is required to keep but cannot write down is not a seam, it is a
/// leak. A caller that mints one on every start instead of storing it grows a node in the account
/// per launch, against a per-user cap the server really enforces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeToken {
    pub node_id: String,
    pub slug: String,
    pub token: String,
}

impl NodeToken {
    pub fn as_str(&self) -> &str {
        &self.token
    }
}

impl AsRef<str> for NodeToken {
    fn as_ref(&self) -> &str {
        &self.token
    }
}

impl Account {
    /// Mint a node token against this account.
    ///
    /// One account credential can stand up any number of nodes, and each needs its own token: the
    /// server's registry is keyed on the node id the token resolves to, so two connections holding
    /// the same token are one node, the later one displacing the earlier.
    pub async fn register_node(&self, spec: NodeSpec) -> Result<NodeToken, RegisterError> {
        let bearer = self.bearer().await.map_err(|error| match error {
            // A dead grant survives as a dead grant: minting under it will keep failing, and a
            // caller told "unreachable" would retry until someone looked at a log.
            EnrollError::Revoked => RegisterError::Revoked,
            EnrollError::Unreachable(transport) => RegisterError::Unreachable(transport),
            other => RegisterError::Unreachable(TransportError::Io(other.to_string())),
        })?;

        let response = self
            .http
            .post(format!("{}/zyris/v1/device/nodes", self.base_url))
            .bearer_auth(&bearer)
            .json(&serde_json::json!({
                "name": spec.name,
                "platform": spec.platform,
                "scopes": spec.scopes,
            }))
            .send()
            .await
            .map_err(|error| RegisterError::Unreachable(TransportError::Io(error.to_string())))?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(refusal(status, &body).unwrap_or_else(|| {
                RegisterError::Unreachable(TransportError::Io(format!("http_{status}: {body}")))
            }));
        }

        let issued: IssuedNode = response
            .json()
            .await
            .map_err(|error| RegisterError::Unreachable(TransportError::Io(error.to_string())))?;
        Ok(NodeToken { node_id: issued.node_id, slug: issued.slug, token: issued.token })
    }
}

#[derive(Deserialize)]
struct IssuedNode {
    node_id: String,
    slug: String,
    token: String,
}

/// The two refusals a caller can act on, told apart by the body rather than by the status alone: a
/// 403 from an ingress is not this account saying no, and answering it as if it were sends someone
/// to their settings page over a proxy hiccup.
fn refusal(status: u16, body: &str) -> Option<RegisterError> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    match value.get("error")?.as_str()? {
        "forbidden_scope" if status == 403 => Some(RegisterError::Forbidden),
        "scope_exceeded" if status == 422 => Some(RegisterError::ScopeExceeded {
            requested: strings(value.get("requested")),
            granted: strings(value.get("granted")),
        }),
        _ => None,
    }
}

fn strings(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential() -> AccountCredential {
        AccountCredential::new(
            "zna_access".into(),
            "znr_refresh".into(),
            "node-id".into(),
            "hello node".into(),
            "allen@example.com".into(),
            1_000_000,
        )
    }

    /// `bearer` immediately before each dial is the whole mid-connection-expiry story, so the
    /// skew boundary is the thing worth pinning.
    #[test]
    fn a_token_expires_early_by_the_skew_allowance() {
        let credential = credential();
        assert_eq!(credential.bearer(0, 30), Some("zna_access"));
        assert_eq!(credential.bearer(999_969, 30), Some("zna_access"));
        // 30 seconds out with a 30-second allowance: refuse, rather than race the handshake.
        assert_eq!(credential.bearer(999_970, 30), None);
        assert_eq!(credential.bearer(2_000_000, 30), None);
    }

    #[test]
    fn a_refresh_is_due_at_eighty_percent_of_the_lifetime() {
        let credential = credential();
        // A one-hour token expiring at 1_000_000 was issued at 996_400; 80% of the way is 999_280.
        assert!(!credential.should_refresh(999_279, 3600));
        assert!(credential.should_refresh(999_280, 3600));
    }

    use crate::enroll::device::scripted_responder;
    use crate::{RegisterError, RotateError};

    const ROTATED: &str = r#"{"access_token":"zna_rotated","refresh_token":"znr_rotated","expires_in":3600,"scope":"agents:read","node_id":"node-7","node_name":"hello node","owner_email":"allen@example.com"}"#;

    fn at(seconds_from_now: i64) -> AccountCredential {
        AccountCredential::new(
            "zna_old".into(),
            "znr_old".into(),
            "node-7".into(),
            "hello node".into(),
            "allen@example.com".into(),
            crate::enroll::device::now_unix() + seconds_from_now,
        )
    }

    /// Past the 80% mark, so a rotation is due — and still presentable, which is the whole point of
    /// rotating early: there is something to carry on with when the rotation cannot be kept.
    fn aging() -> AccountCredential {
        at(300)
    }

    /// Nine tenths of its life left: nothing should be asked of the server at all.
    fn fresh() -> AccountCredential {
        at(3300)
    }

    /// Spent. Nothing left to present, so a rotation that cannot be kept has to be reported.
    fn spent() -> AccountCredential {
        at(-10)
    }

    /// **The property that keeps a node alive.** A refresh token is single-use. If this process
    /// starts presenting a rotated pair that never reached the caller's storage, the next start
    /// presents the spent one, and attacca reads a replay past its 30-second grace as a leaked
    /// chain — `revoke_all_for_node`, which kills every node under the credential.
    #[tokio::test]
    async fn a_rotation_the_hook_could_not_store_is_never_the_one_presented() {
        let url = scripted_responder(&[("/device/refresh", "200 OK", ROTATED)]);
        let account = Account::restore(&url, aging())
            .on_rotate(|_| async { Err(RotateError("the disk is full".into())) })
            .build();

        assert_eq!(
            account.bearer().await.expect("the token in hand is still good"),
            "zna_old",
            "a pair the caller could not store must never go out on the wire"
        );
        assert_eq!(
            account.credential().await.refresh_token,
            "znr_old",
            "and the refresh token this process holds is still the one the caller has on disk"
        );
    }

    /// The other half: once the hook says it is kept, that pair is the one in use, and it is
    /// exactly the pair the hook was handed.
    #[tokio::test]
    async fn the_pair_the_hook_stored_is_the_pair_the_next_dial_presents() {
        let url = scripted_responder(&[("/device/refresh", "200 OK", ROTATED)]);
        let saved = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = saved.clone();
        let account = Account::restore(&url, aging())
            .on_rotate(move |credential| {
                let sink = sink.clone();
                async move {
                    *sink.lock().expect("saved mutex poisoned") = Some(credential);
                    Ok(())
                }
            })
            .build();

        assert_eq!(account.bearer().await.expect("the rotation was kept"), "zna_rotated");

        let stored = saved.lock().expect("saved mutex poisoned").clone();
        assert_eq!(
            stored,
            Some(account.credential().await),
            "what the caller stored has to be what this process holds, to the field"
        );
    }

    /// The rotation fires at 80% of the lifetime, not at every dial. A token with life left is
    /// presented as it is — the refresh route here would answer with a different one, so reaching
    /// it at all would show.
    #[tokio::test]
    async fn a_token_with_life_left_is_presented_without_asking_for_another() {
        let url = scripted_responder(&[("/device/refresh", "200 OK", ROTATED)]);
        let account = Account::restore(&url, fresh()).on_rotate(|_| async { Ok(()) }).build();

        assert_eq!(account.bearer().await.expect("nothing to do"), "zna_old");
        assert_eq!(account.credential().await.access_token, "zna_old");
    }

    /// When there is nothing left to present, a hook that refuses is the caller's problem and has
    /// to say so — silently handing back a spent token would fail at the handshake instead, where
    /// the reason is gone.
    #[tokio::test]
    async fn a_rotation_that_cannot_be_stored_is_reported_when_nothing_is_left_to_present() {
        let url = scripted_responder(&[("/device/refresh", "200 OK", ROTATED)]);
        let account = Account::restore(&url, spent())
            .on_rotate(|_| async { Err(RotateError("read-only filesystem".into())) })
            .build();

        let error = account.bearer().await.expect_err("there is no token left to hand back");
        assert!(
            error.to_string().contains("read-only filesystem"),
            "the caller has to learn why, got: {error}"
        );
        assert_eq!(
            account.credential().await.refresh_token,
            "znr_old",
            "and the rotation still is not adopted"
        );
    }

    /// A responder that keeps the request lines it was sent. What authorizes minting a node is the
    /// whole question here, and it travels in a header rather than in the body.
    fn recording_responder(
        scripts: &'static [(&'static str, &'static str, &'static str)],
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
        let address = listener.local_addr().expect("no local address");
        let recorded = seen.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.read(&mut buf) else { continue };
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                recorded.lock().expect("recording mutex poisoned").push(head);

                let (status, body) = scripts
                    .iter()
                    .find(|(suffix, _, _)| path.contains(suffix))
                    .map(|(_, status, body)| (*status, *body))
                    .unwrap_or(("404 Not Found", r#"{"error":"nothing scripted for this path"}"#));
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("ws://{address}/zyris/v1/ws"), seen)
    }

    /// Answers in the order the requests arrive, and keeps each one whole — head and body.
    ///
    /// [`recording_responder`] above cannot stand in for this. It is keyed by path suffix and
    /// answers the same body every time that path is asked for, so two registrations — which are
    /// two POSTs to `/device/nodes` — would come back as one node id no matter what
    /// `register_node` did with them. It also reads a single chunk, which is enough to see a
    /// request line but not enough to see what was serialized.
    fn ordered_responder(
        answers: &'static [(&'static str, &'static str)],
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
        let address = listener.local_addr().expect("no local address");
        let recorded = seen.clone();
        std::thread::spawn(move || {
            use std::io::Write;
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                // Recorded before the answer goes out, so a caller that has its result back is a
                // caller whose request is already in the log.
                recorded.lock().expect("recording mutex poisoned").push(whole_request(&mut stream));

                // The last answer repeats. A test that makes one more call than it scripted then
                // gets a reply rather than a hang, and a hung suite is the failure that costs an
                // hour: `cargo` dies and the test binary keeps a core.
                let (status, body) = answers[index.min(answers.len() - 1)];
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("ws://{address}/zyris/v1/ws"), seen)
    }

    /// Read until the whole body has arrived rather than until the first chunk has.
    fn whole_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw).to_string();
            let Some(head_end) = text.find("\r\n\r\n") else { continue };
            if raw.len() >= head_end + 4 + content_length(&text[..head_end]) {
                break;
            }
        }
        String::from_utf8_lossy(&raw).to_string()
    }

    fn content_length(head: &str) -> usize {
        head.lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().to_string())
            })
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    fn spec() -> NodeSpec {
        NodeSpec {
            name: "arch zyris-code".into(),
            platform: Some("linux".into()),
            scopes: vec!["agents:read".into()],
        }
    }

    /// One account credential, any number of nodes. Each mint has to be authorized by the account
    /// itself — a static node token minting another is the rule this route does not break.
    #[tokio::test]
    async fn a_node_token_is_minted_against_the_accounts_own_access_token() {
        let (url, seen) = recording_responder(&[(
            "/device/nodes",
            "200 OK",
            r#"{"node_id":"node-9","slug":"arch-zyris-code","token":"znt_minted"}"#,
        )]);
        let account = Account::restore(&url, fresh()).build();

        let token = account.register_node(spec()).await.expect("the server minted one");

        assert_eq!(token.node_id, "node-9");
        assert_eq!(token.slug, "arch-zyris-code");
        assert_eq!(token.as_str(), "znt_minted");
        // The token is what a caller hands to `Node::connect`, so it has to work as a plain string.
        assert_eq!(AsRef::<str>::as_ref(&token), "znt_minted");

        let request = seen.lock().expect("recording mutex poisoned").join("\n").to_lowercase();
        assert!(
            request.contains("authorization: bearer zna_old"),
            "the account credential is what authorizes a mint: {request}"
        );
    }

    /// Scopes are clamped to the account's grant, and the caller can only drop the ones it asked
    /// for too many of if it is told which those were.
    #[test]
    fn the_two_refusals_are_read_from_the_body_and_not_from_the_status_alone() {
        assert!(matches!(
            refusal(403, r#"{"error":"forbidden_scope"}"#),
            Some(RegisterError::Forbidden)
        ));

        match refusal(
            422,
            r#"{"error":"scope_exceeded","requested":["nodes:write","agents:read"],"granted":["agents:read"]}"#,
        ) {
            Some(RegisterError::ScopeExceeded { requested, granted }) => {
                assert_eq!(requested, vec!["nodes:write".to_string(), "agents:read".to_string()]);
                assert_eq!(granted, vec!["agents:read".to_string()]);
            }
            other => panic!("the names have to come back, got {other:?}"),
        }

        // An ingress refusing the request said nothing about this account's grant.
        assert!(refusal(403, "<html>Forbidden</html>").is_none());
        assert!(refusal(422, r#"{"error":"forbidden_scope"}"#).is_none());
    }

    /// And the same refusal end to end, because reading a body is only half of it — the status has
    /// to reach the parsing at all rather than being swallowed as a transport failure.
    #[tokio::test]
    async fn a_credential_that_may_not_mint_nodes_says_so_rather_than_looking_unreachable() {
        let (url, _seen) = recording_responder(&[(
            "/device/nodes",
            "403 Forbidden",
            r#"{"error":"forbidden_scope"}"#,
        )]);
        let account = Account::restore(&url, fresh()).build();

        let error = account.register_node(spec()).await.expect_err("this grant may not mint");

        assert!(
            matches!(error, RegisterError::Forbidden),
            "a caller that reads this as unreachable retries forever, got {error}"
        );
    }

    /// **The feature, in one assertion.** The server's registry is `insert(node_id, connection)`
    /// (`attacca-zyris/src/registry.rs`), so two links presenting one token displace each other —
    /// the later one closes the earlier as superseded. Two windows of one program can both serve
    /// tools only if each holds a node of its own, and this call is what hands them out.
    ///
    /// The wrong implementation it guards against is a near one, not a hypothetical:
    /// `AccountCredential` carries a `node_id` too, and answering with that gives back a
    /// well-formed `NodeToken` under which every node on the account is the same node.
    #[tokio::test]
    async fn two_registrations_on_one_account_are_two_different_nodes() {
        let (url, seen) = ordered_responder(&[
            ("200 OK", r#"{"node_id":"node-one","slug":"arch-one","token":"znt_first"}"#),
            ("200 OK", r#"{"node_id":"node-two","slug":"arch-two","token":"znt_second"}"#),
        ]);
        let account = Account::restore(&url, fresh()).build();

        let first = account
            .register_node(NodeSpec { name: "arch one".into(), ..spec() })
            .await
            .expect("the first registration succeeds");
        let second = account
            .register_node(NodeSpec { name: "arch two".into(), ..spec() })
            .await
            .expect("the second registration succeeds");

        assert_ne!(first.node_id, second.node_id, "each registration is a node of its own");
        assert_ne!(first.as_str(), second.as_str(), "and each node carries its own token");

        // Neither of them is the account's own node. That confusion is what the two layers exist
        // to end, so it is worth naming the value being ruled out rather than implying it.
        assert_eq!(account.credential().await.node_id, "node-7");
        assert_ne!(first.node_id, "node-7", "a registered node is a sibling, not the account");
        assert_ne!(second.node_id, "node-7");

        // Two registrations, not one asked for twice: each carried the name it was given, which is
        // only visible because this responder keeps the request body.
        let requests = seen.lock().expect("recording mutex poisoned");
        assert_eq!(requests.len(), 2, "one request each, got {}", requests.len());
        assert!(requests[0].contains(r#""name":"arch one""#), "got:\n{}", requests[0]);
        assert!(requests[1].contains(r#""name":"arch two""#), "got:\n{}", requests[1]);
    }

    /// The library hands this out once and keeps no copy, so a caller that cannot write it down has
    /// to mint a new one every start — and every mint is another node in the account, against a cap
    /// the server enforces. Serializing it is therefore part of the contract, not a convenience.
    #[test]
    fn a_node_token_can_be_written_down_and_read_back() {
        let minted = NodeToken {
            node_id: "01J0NODE".to_string(),
            slug: "arch-zyris-code".to_string(),
            token: "znt_kept_by_the_caller".to_string(),
        };

        let written = serde_json::to_string(&minted).expect("a node token serializes");
        let read_back: NodeToken = serde_json::from_str(&written).expect("and reads back");

        assert_eq!(read_back, minted, "what comes back has to be the identity that went in");
    }

}
