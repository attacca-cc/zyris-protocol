//! A dialler acts on *why* a connection failed, and until now it could not: every refusal arrived
//! as one string, so `zyris-daemon` grew a `gave_up: AtomicBool` to guess at the one distinction
//! that matters — a credential the server has disowned, which needs a person, against a server
//! having a bad day, which needs another try.
//!
//! The two mistakes cost different things. Reading an outage as a revocation drags somebody back
//! to a terminal to approve a code; reading a revocation as an outage is a node that reconnects
//! forever and is never coming back.

use zyris::ConnectError;
use zyris::EnrollError;

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
    let Err(refused) = zyris::transport::ws::connect(&url, "zc_wrong").await else {
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
    let Err(refused) = zyris::transport::ws::connect(&url, "zc_fine").await else {
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
    let Err(failed) = zyris::transport::ws::connect(&url, "zc_fine").await else {
        panic!("nothing is listening on that port")
    };

    assert!(
        matches!(ConnectError::from(failed), ConnectError::Unreachable(_)),
        "a closed port says nothing about the credential"
    );
}

/// The scope the server did not recognise has to arrive as a name. Today's 422 is a serde dump, so
/// a node can only report that *something* was wrong with a list it wrote itself.
#[test]
fn an_unknown_scope_comes_back_by_name() {
    let refused = EnrollError::ScopeUnknown { scope: "nodes:write".to_string() };
    assert!(refused.to_string().contains("nodes:write"), "got {refused}");
}
