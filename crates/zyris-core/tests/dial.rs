//! What a refused dial means, told apart by type rather than by reading a string.
//!
//! The distinction is the one a node acts on. A credential the server has disowned needs a person,
//! and no amount of retrying substitutes for one; a server having a bad day needs exactly retrying
//! and nothing else. Collapsing the two into one `WireError` is what made a consumer of this crate
//! carry an `AtomicBool` to guess which of them had happened.
//!
//! `connect_errors.rs` covers the same classification one layer down, over
//! `transport::ws::connect`. These tests are over the entry point a node author actually calls, so
//! a rename or a rewiring there cannot quietly stop applying it.

use zyris::proto::{
    decode_binary, encode_control, AckProtocol, Envelope, HelloAck, IncomingFrame, Serialization,
};
use zyris::transport::{ChannelTransport, Transport};
use zyris::{ConnectError, Node, NodeKind};

fn probe() -> Node {
    Node::builder().name("probe").kind(NodeKind::Cli).build().unwrap()
}

/// A server that answers the websocket upgrade with a refusal, on a fresh loopback port.
///
/// A real socket rather than a stubbed client: what `dial` has to classify is an HTTP status line,
/// and stubbing the client would stub out the one input the classification reads.
fn refusing_server(status_line: &'static str, body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
    let address = listener.local_addr().expect("no local address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            use std::io::{Read, Write};
            let Ok(mut stream) = stream else { return };
            let _ = stream.read(&mut [0u8; 4096]);
            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("ws://{address}/zyris/v1/ws")
}

#[tokio::test]
async fn a_credential_the_server_refuses_is_not_reported_as_a_network_problem() {
    let refusing = refusing_server("401 Unauthorized", r#"{"error":"unauthorized"}"#);
    let error = probe()
        .dial(&refusing, "znt_wrong")
        .await
        .err()
        .expect("a 401 upgrade is not a connection");
    assert!(
        matches!(error, ConnectError::Unauthorized),
        "a refused token has to be tellable from an unreachable server, got: {error}"
    );
}

#[tokio::test]
async fn a_server_having_a_bad_day_is_not_reported_as_a_dead_credential() {
    let flaky = refusing_server("503 Service Unavailable", r#"{"error":"unavailable"}"#);
    let error =
        probe().dial(&flaky, "znt_fine").await.err().expect("a 503 upgrade is not a connection");
    assert!(
        matches!(error, ConnectError::Unreachable(_)),
        "a deploy must not be able to unenroll a fleet, got: {error}"
    );
}

/// A 426 refuses the upgrade before either side has spoken Zyris, so the server's version is not
/// on the wire at all. The one side that can be named is named, and the other says nothing rather
/// than carrying a number somebody invented to fill the field — a version printed in a log is read
/// as fact.
#[tokio::test]
async fn a_refusal_before_the_protocol_was_spoken_leaves_the_peers_version_unstated() {
    let outdated = refusing_server("426 Upgrade Required", r#"{"error":"upgrade_required"}"#);
    let error =
        probe().dial(&outdated, "znt_fine").await.err().expect("a 426 upgrade is not a connection");
    match error {
        ConnectError::VersionMismatch { ours, theirs } => {
            assert_eq!(
                ours,
                zyris::proto::PROTOCOL_MAJOR.to_string(),
                "the side that can be named has to be named"
            );
            assert_eq!(
                theirs, None,
                "nothing about the server's version reached us, so nothing may be reported"
            );
        }
        other => panic!("an upgrade refusal is a version mismatch, got: {other}"),
    }
}

/// The other half of the same field. Once the handshake is under way the peer states its major in
/// its `HelloAck`, and that number has to survive into the typed error — a caller that has to
/// re-read it out of the message's prose is coupled to the wording of a sentence this crate is
/// free to rewrite.
///
/// `dial` is `transport::ws::connect` followed by `connect_over`; this drives the second half of
/// it directly, because reaching the handshake over a real socket would mean standing up a server
/// that speaks Zyris only far enough to disagree about the version.
#[tokio::test]
async fn a_peer_that_answered_the_handshake_has_its_major_named_in_the_mismatch() {
    let (dial_side, accept_side) = ChannelTransport::pair();
    let ahead = zyris::proto::PROTOCOL_MAJOR + 1;

    tokio::spawn(async move {
        let (mut sink, mut stream) = Box::new(accept_side).split();
        let hello = stream.next().await.expect("the dialler says hello first").expect("a frame");
        let zyris::proto::WireMessage::Binary(bytes) = hello else {
            panic!("the handshake is msgpack")
        };
        assert!(
            matches!(decode_binary(bytes), Ok(IncomingFrame::Control(Envelope::Hello(_)))),
            "the dialler opens with a hello"
        );
        let ack = Envelope::HelloAck(HelloAck {
            protocol: AckProtocol { major: ahead, minor: 0 },
            serialization: Serialization::Msgpack,
            conn_id: "c1".to_string(),
            resume_token: String::new(),
            node_id: "n1".to_string(),
            heartbeat: Default::default(),
            limits: Default::default(),
            resumed: false,
            features: Vec::new(),
        });
        let encoded = encode_control(&ack, Serialization::Msgpack).expect("the ack encodes");
        let _ = sink.send(encoded).await;
    });

    let refused = probe()
        .connect_over(dial_side)
        .await
        .err()
        .expect("a peer speaking another major is not a connection");
    match ConnectError::from(refused) {
        ConnectError::VersionMismatch { theirs, .. } => assert_eq!(
            theirs,
            Some(ahead.to_string()),
            "the peer said which major it speaks, so the error has to carry it"
        ),
        other => panic!("a disagreement about the major is a version mismatch, got: {other}"),
    }
}

/// The rename touched the entry point, not the handshake under it. Without this, a failure in the
/// tests above could be a connection that stopped working rather than a misclassification.
#[tokio::test]
async fn the_in_process_handshake_still_comes_up_after_the_rename() {
    let (dial_conn, accept_conn) =
        zyris::testing::duplex(&probe(), &probe()).await.expect("the duplex handshake completes");
    assert!(!dial_conn.is_closed());
    assert!(!accept_conn.is_closed());
}

/// `connect` is the maintained form of `dial` and has to answer the same way when the server says
/// no. A link that retried a refused token forever would be a node that never reports it needs a
/// person — which is the whole reason the refusal is typed.
#[tokio::test]
async fn a_token_the_server_refuses_never_becomes_a_link() {
    let refusing = refusing_server("401 Unauthorized", r#"{"error":"unauthorized"}"#);
    let node = Node::builder()
        .name("probe")
        .kind(NodeKind::Cli)
        .on_connect(|_conn| async move {})
        .build()
        .unwrap();

    let error = node
        .connect(&refusing, String::from("znt_wrong"))
        .await
        .err()
        .expect("a 401 upgrade is not a link");
    assert!(matches!(error, ConnectError::Unauthorized), "got: {error}");
}
