//! What a build that selected no TLS provider does when it is asked to dial.
//!
//! Rustls chooses its provider from crate features and, when it cannot, panics from
//! `CryptoProvider::get_default_or_install_from_crate_features()`. There is no `compile_error!`
//! anywhere in rustls, so the failure is deferred all the way to the first connection: `cargo add
//! zyris` compiles on a bare machine, the node comes up, announces nothing yet — and dies on its
//! first `wss://`. `zyris-p2p`'s `tls` module already records that exact panic for the case where
//! *two* providers are linked and the choice is ambiguous. Linking *zero* is the default graph of
//! this crate, and it reaches the same line.
//!
//! So this crate answers the question itself, before a socket is opened, and answers it with a
//! type. The distinction that matters is the one `is_fatal` reads: a missing provider is a
//! property of the build, and no amount of retrying reaches one.
//!
//! The gate has two directions and both are here, each compiled by the build it describes: a
//! build with no provider must refuse, and a build with one must not. Gating the file as a whole
//! would leave the second unwritten, and refusing a dial that could have gone through is the
//! failure that would not show up until someone else's node stopped connecting.

#![cfg(feature = "client")]

use zyris_core::{ConnectError, Node, NodeKind};

fn probe() -> Node {
    Node::builder().name("probe").kind(NodeKind::Cli).build().unwrap()
}

/// Port 1 has nothing listening on it. If the answer below were reached over the network it would
/// be a refused connection — which is the wrong answer twice over: it names the network for a
/// fault in the build, and `is_fatal` would send the reconnect loop back to try it again forever.
#[cfg(not(any(feature = "tls-ring", feature = "tls-aws-lc")))]
#[tokio::test]
async fn dialling_wss_without_a_tls_provider_is_refused_by_name() {
    let Err(error) = probe().dial("wss://127.0.0.1:1/", "znt_whatever").await else {
        panic!("nothing is listening on port 1, so this dial cannot have succeeded");
    };

    assert!(
        matches!(error, ConnectError::NoTlsProvider),
        "a build that selected no TLS provider should say so, got {error:?}"
    );

    let said = error.to_string();
    assert!(said.contains("tls-ring"), "the message must name a way out, got {said:?}");
    assert!(said.contains("tls-aws-lc"), "the message must name both providers, got {said:?}");
}

/// The other half of the promise the gate makes in `node.rs` — "before the socket, because this is
/// a fact about the build and the network has no say in it". The test above dials a port with
/// nothing on it, where a gate that ran *after* the socket would give an indistinguishable answer,
/// so until this test the ordering was asserted by a comment and by nothing else. It is the half
/// that matters for `is_fatal`: a refusal decided from the build is one no reconnect loop can talk
/// its way past, and a refusal decided from the network is one it will retry forever.
///
/// Here something *is* listening, and never accepts. A TCP handshake into a listening socket is
/// completed by the kernel and waits in the backlog whether or not the program ever asks for it,
/// so a single non-blocking `accept` afterwards settles the question outright — no sleep, no
/// timing, no second thread. `WouldBlock` means nothing ever connected.
#[cfg(not(any(feature = "tls-ring", feature = "tls-aws-lc")))]
#[tokio::test]
async fn the_refusal_is_decided_before_anything_reaches_the_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("no loopback port");
    let address = listener.local_addr().expect("no local address");

    let url = format!("wss://{address}/zyris/v1/ws");
    let Err(error) = probe().dial(&url, "znt_whatever").await else {
        panic!("this build selected no TLS provider, so the dial cannot have succeeded");
    };
    assert!(
        matches!(error, ConnectError::NoTlsProvider),
        "a listening port does not change what refused this, got {error:?}"
    );

    listener.set_nonblocking(true).expect("the listener can be asked without waiting");
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok(_) => panic!(
            "a connection reached the listener, so the provider was checked after a socket had \
             already been opened; the refusal has to be decided from the build alone"
        ),
        Err(e) => panic!("the listener could not be asked whether anything arrived: {e}"),
    }
}

/// The gate is the scheme, not the dial. `ws://` carries no TLS, so a provider is not what stops
/// it — refusing it here would break every plaintext deployment for a reason that does not apply.
#[cfg(not(any(feature = "tls-ring", feature = "tls-aws-lc")))]
#[tokio::test]
async fn a_plaintext_url_needs_no_provider() {
    let Err(error) = probe().dial("ws://127.0.0.1:1/", "znt_whatever").await else {
        panic!("nothing is listening on port 1, so this dial cannot have succeeded");
    };

    assert!(
        !matches!(error, ConnectError::NoTlsProvider),
        "ws:// carries no TLS, so the provider cannot be what refused it, got {error:?}"
    );
}

/// The other direction, and the reason the gate is a question rather than a rule: a build that
/// named a provider must get past it. What stops this dial is port 1 having nothing on it, which
/// is the network's answer to give.
#[cfg(any(feature = "tls-ring", feature = "tls-aws-lc"))]
#[tokio::test]
async fn a_build_that_named_a_provider_gets_past_the_gate() {
    let Err(error) = probe().dial("wss://127.0.0.1:1/", "znt_whatever").await else {
        panic!("nothing is listening on port 1, so this dial cannot have succeeded");
    };

    assert!(
        !matches!(error, ConnectError::NoTlsProvider),
        "a provider was selected at build time, so the gate had no business refusing: {error:?}"
    );
}
