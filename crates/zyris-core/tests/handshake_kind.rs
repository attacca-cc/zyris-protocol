//! What a dialer says it is has to survive the handshake.
//!
//! It has always been *in* the agent string (`zyris/0.1.0 (zyrisd-cli; cli)`), which is a sentence
//! for a log line. An acceptor that treats a consumer differently from a node — Attacca does,
//! because `zyrisd peers` borrows the node's credential to call an API and must not be mistaken
//! for the node itself — should not have to substring-match its way to that decision.

use zyris::proto::{decode_binary, Envelope, Hello, IncomingFrame, WireMessage};
use zyris::transport::{ChannelTransport, Transport};
use zyris::{AcceptOptions, Node, NodeAddress, NodeKind};

fn node_of(kind: NodeKind) -> Node {
    Node::builder().name("probe").kind(kind).build().unwrap()
}

#[tokio::test]
async fn the_acceptor_learns_what_the_dialer_is() {
    for kind in [NodeKind::Cli, NodeKind::Desktop, NodeKind::Server, NodeKind::Service] {
        let dialer = node_of(kind.clone());
        let acceptor = node_of(NodeKind::Server);
        let (dial_conn, accept_conn) = zyris::testing::duplex(&dialer, &acceptor).await.unwrap();

        assert_eq!(
            accept_conn.info().peer_kind.as_deref(),
            Some(kind.as_str()),
            "the acceptor has to be able to tell a {} apart from anything else",
            kind.as_str()
        );

        // The acceptor sends a HelloAck, not a Hello, so the dialing side is told nothing about
        // its peer — the same asymmetry `peer_agent` already has.
        assert_eq!(dial_conn.info().peer_kind, None, "the dialer receives no hello to learn from");
    }
}

/// The distinction the acceptor actually needs: a consumer borrowing a node's credential, beside a
/// node serving on the same one. Both authenticate identically; only this tells them apart.
#[tokio::test]
async fn a_cli_and_a_desktop_are_distinguishable_on_the_same_credential() {
    let acceptor = node_of(NodeKind::Server);

    let (_, from_daemon) = zyris::testing::duplex(&node_of(NodeKind::Desktop), &acceptor)
        .await
        .unwrap();
    let (_, from_cli) = zyris::testing::duplex(&node_of(NodeKind::Cli), &acceptor).await.unwrap();

    assert_ne!(
        from_daemon.info().peer_kind,
        from_cli.info().peer_kind,
        "if these read the same, an acceptor cannot avoid treating the CLI as the node"
    );
    assert_eq!(from_cli.info().peer_kind.as_deref(), Some(NodeKind::Cli.as_str()));
}

/// The builder's name is no longer only prose in the agent string: it is what the acceptor makes
/// the node's address out of, so it has to travel as a field of its own.
#[tokio::test]
async fn the_hello_carries_the_name_the_builder_was_given() {
    let (dial_side, accept_side) = ChannelTransport::pair();
    let dialer = Node::builder().name("myrepo").kind(NodeKind::Service).build().unwrap();
    // Nobody answers, so the dial never completes; it runs beside the read rather than before it.
    let dialing = tokio::spawn(async move { dialer.connect_over(dial_side).await });

    let (_sink, mut stream) = Box::new(accept_side).split();
    let WireMessage::Binary(bytes) = stream.next().await.expect("a hello").expect("a frame") else {
        panic!("the handshake is msgpack")
    };
    let Ok(IncomingFrame::Control(Envelope::Hello(hello))) = decode_binary(bytes) else {
        panic!("the dialler opens with a hello")
    };
    assert_eq!(hello.node_name.as_deref(), Some("myrepo"));
    dialing.abort();
}

/// The acceptor decides the address; the dialer only learns it. Both ends have to agree on it,
/// and a dial the acceptor assigned nothing to — a `cli`, or an acceptor from before the field —
/// reads as `None` rather than as a made-up address.
#[tokio::test]
async fn the_address_the_acceptor_assigns_is_what_both_ends_read() {
    let assigned = NodeAddress {
        system: "laptop".into(),
        program: "zyris-code".into(),
        name: "myrepo-2".into(),
    };
    let options = AcceptOptions { node: Some(assigned.clone()), ..Default::default() };
    let (dial_conn, accept_conn) = zyris::testing::duplex_with(
        &node_of(NodeKind::Service),
        &node_of(NodeKind::Server),
        options,
    )
    .await
    .unwrap();
    assert_eq!(dial_conn.info().node, Some(assigned.clone()));
    assert_eq!(accept_conn.info().node, Some(assigned), "the acceptor keeps what it handed out");

    let (dial_conn, _) =
        zyris::testing::duplex(&node_of(NodeKind::Cli), &node_of(NodeKind::Server)).await.unwrap();
    assert_eq!(dial_conn.info().node, None);
}

/// An acceptor that names nodes has to read the hello first: the address it answers with is made
/// from `Hello.node_name`, which does not exist until the hello does.
#[tokio::test]
async fn an_acceptor_can_answer_with_what_the_hello_said() {
    let (dial_side, accept_side) = ChannelTransport::pair();
    let dialer = Node::builder().name("myrepo").kind(NodeKind::Service).build().unwrap();
    let acceptor = node_of(NodeKind::Server);
    let accepting = acceptor.accept_with(accept_side, |hello: &Hello| {
        let name = hello.node_name.clone().unwrap_or_default();
        async move {
            Ok(AcceptOptions {
                node: Some(NodeAddress {
                    system: "laptop".into(),
                    program: "zyris-code".into(),
                    name,
                }),
                ..Default::default()
            })
        }
    });

    let (dialed, accepted) = tokio::join!(dialer.connect_over(dial_side), accepting);
    let (dialed, _accepted) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(
        dialed.info().node.as_ref().map(NodeAddress::path),
        Some("laptop/zyris-code/myrepo".to_string())
    );
}

/// And one that refuses — a node hello with no name, a user over the live-node cap — says why
/// before it closes, so the dialer reads a reason rather than a dropped socket.
#[tokio::test]
async fn an_acceptor_that_refuses_the_hello_says_why() {
    let (dial_side, accept_side) = ChannelTransport::pair();
    let acceptor = node_of(NodeKind::Server);
    let accepting = acceptor.accept_with(accept_side, |_: &Hello| async {
        Err::<AcceptOptions, _>(zyris::WireError::invalid_params("a node has to say its name"))
    });

    let dialer = node_of(NodeKind::Service);
    let (dialed, accepted) = tokio::join!(dialer.connect_over(dial_side), accepting);
    let refused = dialed.err().expect("a refused hello is not a connection");
    assert_eq!(refused.code, zyris::ErrorCode::InvalidParams);
    assert!(refused.message.contains("say its name"), "{refused:?}");
    assert!(accepted.is_err(), "the acceptor reports the refusal it made");
}
