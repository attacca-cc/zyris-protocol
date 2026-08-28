//! A dialler acts on *why* a connection failed, and until now it could not: every refusal arrived
//! as one string, so `zyris-daemon` grew a `gave_up: AtomicBool` to guess at the one distinction
//! that matters — a credential the server has disowned, which needs a person, against a server
//! having a bad day, which needs another try.
//!
//! The two mistakes cost different things. Reading an outage as a revocation drags somebody back
//! to a terminal to approve a code; reading a revocation as an outage is a node that reconnects
//! forever and is never coming back.

use zyris::enroll::protocol::{classify_refresh_error, ErrorResponse};
use zyris::ConnectError;
use zyris::{EnrollError, RegisterError, RotateError};

/// Answer whatever arrives with one canned status line, then close.
///
/// A real socket rather than a stubbed client: the branch under test reads an HTTP status off a
/// refused upgrade, and stubbing the client would stub out exactly that input.
fn refusing_server(status_line: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
    let address = listener.local_addr().expect("no local address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            use std::io::{Read, Write};
            let Ok(mut stream) = stream else { return };
            let _ = stream.read(&mut [0u8; 4096]);
            let response =
                format!("HTTP/1.1 {status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("ws://{address}/zyris/v1/ws")
}

/// A mistyped or stale token is answered at the upgrade, before the protocol is ever spoken.
/// Retrying it is pointless, and saying "could not reach the server" sends the reader to their
/// network when the answer is in their token.
#[tokio::test]
async fn a_token_the_server_will_not_take_is_not_a_server_that_could_not_be_reached() {
    let url = refusing_server("401 Unauthorized");
    let Err(refused) = zyris::transport::ws::connect(&url, "znt_wrong").await else {
        panic!("the server answered 401; that upgrade cannot have succeeded")
    };

    assert!(
        matches!(ConnectError::from(refused), ConnectError::Unauthorized),
        "a 401 says the token was judged and refused"
    );
}

/// 426 is the other refusal that no retry reaches, and it means something a person can act on:
/// this build is too old for that deployment.
#[tokio::test]
async fn a_deployment_this_build_cannot_speak_to_says_so_in_the_error() {
    let url = refusing_server("426 Upgrade Required");
    let Err(refused) = zyris::transport::ws::connect(&url, "znt_fine").await else {
        panic!("the server answered 426; that upgrade cannot have succeeded")
    };

    let error = ConnectError::from(refused);
    let ConnectError::VersionMismatch { ours, .. } = &error else {
        panic!("expected a version mismatch, got {error}")
    };
    assert_eq!(ours, "1", "the version this build speaks has to be in the message");
    assert!(error.to_string().contains("protocol mismatch"), "got {error}");
}

/// And the one that a retry *does* reach. Nothing was refused here — nobody answered.
#[tokio::test]
async fn a_socket_that_never_opened_is_reported_as_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
    let address = listener.local_addr().expect("no local address");
    drop(listener);

    let url = format!("ws://{address}/zyris/v1/ws");
    let Err(failed) = zyris::transport::ws::connect(&url, "znt_fine").await else {
        panic!("nothing is listening on that port")
    };

    assert!(
        matches!(ConnectError::from(failed), ConnectError::Unreachable(_)),
        "a closed port says nothing about the credential"
    );
}

fn refusal(error: &str, status: u16) -> ErrorResponse {
    ErrorResponse {
        error: error.to_string(),
        error_description: Some("because".to_string()),
        interval: None,
        status: Some(status),
    }
}

/// The `AtomicBool` this replaces. Both of these carry `invalid_grant` in the body; only the one
/// the server actually answered with means the grant is dead. A rollout that answers 500 for a
/// minute must not unenroll every node that dialled through it.
#[test]
fn a_five_hundred_during_a_deploy_is_not_reported_as_a_revoked_credential() {
    let outage = ConnectError::from(classify_refresh_error(&refusal("invalid_grant", 500)));
    assert!(
        matches!(outage, ConnectError::Unreachable(_)),
        "a server that could not answer has not told us the grant is dead, got {outage}"
    );

    let disowned = ConnectError::from(classify_refresh_error(&refusal("invalid_grant", 400)));
    assert!(
        matches!(disowned, ConnectError::Revoked),
        "the one answer that needs a person, got {disowned}"
    );
    assert!(
        disowned.to_string().contains("authorize this node again"),
        "the message has to say what the person must do, got {disowned}"
    );
}

/// The scope the server did not recognise has to arrive as a name. Today's 422 is a serde dump, so
/// a node can only report that *something* was wrong with a list it wrote itself.
#[test]
fn an_unknown_scope_comes_back_by_name() {
    let refused = EnrollError::ScopeUnknown { scope: "nodes:write".to_string() };
    assert!(refused.to_string().contains("nodes:write"), "got {refused}");
}

/// Which scopes were clamped is a set difference, and a caller must be able to take it without
/// parsing a sentence.
#[test]
fn a_clamped_registration_says_which_scopes_it_could_not_grant() {
    let refused = RegisterError::ScopeExceeded {
        requested: vec!["agents:read".to_string(), "nodes:write".to_string()],
        granted: vec!["agents:read".to_string()],
    };

    let RegisterError::ScopeExceeded { requested, granted } = &refused else {
        panic!("expected a clamp, got {refused}")
    };
    let refused_scopes: Vec<&str> =
        requested.iter().filter(|s| !granted.contains(s)).map(String::as_str).collect();
    assert_eq!(refused_scopes, ["nodes:write"]);
}

/// The library hands a rotated credential to the caller and will not use it until the caller says
/// it is stored. When storing fails, why it failed is the caller's own words — this crate has no
/// idea what a keychain, a Secret, or a database is.
#[test]
fn a_store_that_refused_a_rotation_keeps_its_own_words() {
    let failed = RotateError("the keychain is locked".to_string());
    assert!(failed.to_string().contains("the keychain is locked"), "got {failed}");
}
