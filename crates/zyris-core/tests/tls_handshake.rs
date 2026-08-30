//! A `wss://` dial that completes a real TLS handshake and speaks Zyris over it.
//!
//! Every other loopback responder in this workspace is plaintext, and every `wss://` elsewhere in
//! the tree is a string literal inside a URL-derivation or an error-text assertion. So until this
//! file existed, `tls.rs` — `install_chosen_provider`, `provider_is_missing`, `missing_for` — had
//! never met a socket, and the default deployment is `wss://attacca.cc`. The half of the TLS story
//! that was tested was the half where TLS does *not* happen.
//!
//! What is proven here is one dial through the unmodified production path: that the provider this
//! build selected is one rustls can actually negotiate with rather than merely one
//! `get_default()` reports as present, that the `authorization` header `ws::connect` adds is
//! written inside the TLS session, and that the msgpack handshake and a capability call cross it.
//! Reaching `Ok(Connection)` is already the protocol and not just an HTTP upgrade: the Zyris
//! handshake is what turns a websocket into a connection.
//!
//! **No production code has a test seam in it, and that is deliberate.** The trust anchor is
//! swapped with `SSL_CERT_FILE`, which is documented public behaviour of `rustls-native-certs` —
//! the crate `tokio-tungstenite` calls on exactly the code path a real dial takes, under the
//! `rustls-tls-native-roots` feature this crate turns on. It *replaces* the platform store rather
//! than adding to it, so the test cannot pass because some real CA vouched for something, and it
//! works the same on Linux, macOS and Windows. A knob of our own would have been a knob a
//! deployment could find, and a way for a deployment to weaken certificate checking is worse than
//! the missing test was.
//!
//! The second dial is not decoration. A handshake test that only ever presents a certificate it
//! trusts passes just as well against a verifier that has been switched off, so the same client
//! is pointed at a server holding a certificate from a CA it has never heard of and has to refuse
//! it. One test rather than two, because `SSL_CERT_FILE` is process-global and two tests in one
//! binary would be two threads writing it.
//!
//! What this still does not prove is written down at the bottom of the file.

#![cfg(all(feature = "client", any(feature = "tls-ring", feature = "tls-aws-lc")))]

use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use zyris::proto::WireMessage;
use zyris::transport::{Transport, WireSink, WireStream};
use zyris::{AcceptOptions, ConnectError, Node, NodeKind, TransportError};

// --- something to say over the connection, so the assertion is about the protocol -------------

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pong {
    pub said: String,
}

#[zyris::capability(name = "over_tls", version = 1)]
pub trait OverTls {
    /// Answer with what was said.
    async fn echo(&self, said: String) -> zyris::Result<Pong>;
}

struct Echo;

#[zyris::async_trait]
impl OverTls for Echo {
    async fn echo(&self, said: String) -> zyris::Result<Pong> {
        Ok(Pong { said })
    }
}

// --- certificates, generated per run ----------------------------------------------------------

/// A certificate authority and the material to sign leaves with it.
///
/// Generated rather than checked in for two reasons that are both about the repository rather
/// than about TLS: a checked-in fixture puts a private key in a public tree, and it puts an expiry
/// date in the suite that turns it red on a day nobody chose. rcgen's defaults run from 1975 to
/// 4096, so nothing here depends on the machine's clock being right either.
struct TestCa {
    pem: String,
    issuer: Issuer<'static, KeyPair>,
}

impl TestCa {
    fn new(common_name: &str) -> TestCa {
        let key = KeyPair::generate().expect("a key pair");
        let mut params = CertificateParams::new(Vec::new()).expect("no SANs on a CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.distinguished_name.push(rcgen::DnType::CommonName, common_name);
        let certificate = params.self_signed(&key).expect("a self-signed CA");
        TestCa { pem: certificate.pem(), issuer: Issuer::new(params, key) }
    }

    /// A leaf for `127.0.0.1`. rcgen reads an address-shaped SAN as `SanType::IpAddress` itself,
    /// which is what the handshake needs: the server name rustls is given comes from the URL's
    /// host, and a loopback URL has no name in it to match against.
    fn loopback_leaf(&self) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().expect("a key pair");
        let mut params =
            CertificateParams::new(vec!["127.0.0.1".to_string()]).expect("a loopback SAN");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let certificate = params.signed_by(&key, &self.issuer).expect("a signed leaf");
        let chain = vec![certificate.der().clone()];
        (chain, PrivateKeyDer::Pkcs8(key.serialize_der().into()))
    }
}

/// The provider the *server* uses, named by the same cfg ladder `tls.rs` uses for the client so
/// the two halves cannot end up disagreeing about which one this build has.
///
/// Deliberately `builder_with_provider` and never `install_default`: installing one here would put
/// it where the client's `ClientConfig::builder()` reads from, and the test would be supplying the
/// very thing it is meant to be checking `install_chosen_provider` supplied.
#[cfg(feature = "tls-aws-lc")]
fn server_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[cfg(all(feature = "tls-ring", not(feature = "tls-aws-lc")))]
fn server_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

// --- the server side of the socket ------------------------------------------------------------

type ServerWs = WebSocketStream<TlsStream<TcpStream>>;

/// A [`Transport`] over the server end of a TLS websocket.
///
/// `ws::WsTransport` cannot stand in for this: its stream is a `MaybeTlsStream` holding a *client*
/// `TlsStream`, and its field is private. Without an adapter the most this test could reach is an
/// HTTP status decrypted over TLS, which proves the handshake and nothing about the protocol.
struct TlsServerTransport(ServerWs);

struct TlsServerSink(SplitSink<ServerWs, Message>);
struct TlsServerRead(SplitStream<ServerWs>);

#[zyris::async_trait]
impl WireSink for TlsServerSink {
    async fn send(&mut self, msg: WireMessage) -> Result<(), TransportError> {
        let message = match msg {
            WireMessage::Binary(b) => Message::Binary(b),
            WireMessage::Text(t) => Message::Text(t.into()),
        };
        self.0.send(message).await.map_err(|e| TransportError::Io(e.to_string()))
    }

    async fn close(&mut self, code: u16, reason: String) -> Result<(), TransportError> {
        let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
        let _ = self.0.send(Message::Close(Some(frame))).await;
        Ok(())
    }
}

#[zyris::async_trait]
impl WireStream for TlsServerRead {
    async fn next(&mut self) -> Option<Result<WireMessage, TransportError>> {
        loop {
            match self.0.next().await {
                None => return None,
                Some(Err(e)) => return Some(Err(TransportError::Io(e.to_string()))),
                Some(Ok(Message::Binary(b))) => return Some(Ok(WireMessage::Binary(b))),
                Some(Ok(Message::Text(t))) => return Some(Ok(WireMessage::Text(t.to_string()))),
                Some(Ok(Message::Close(_))) => return None,
                Some(Ok(_)) => continue,
            }
        }
    }
}

impl Transport for TlsServerTransport {
    fn split(self: Box<Self>) -> (Box<dyn WireSink>, Box<dyn WireStream>) {
        let (sink, stream) = self.0.split();
        (Box::new(TlsServerSink(sink)), Box::new(TlsServerRead(stream)))
    }
}

/// Stand a TLS listener up on a fresh loopback port and answer one connection with a Zyris node.
///
/// Bound synchronously before anything is spawned, the way `dial.rs` does it, so the port in the
/// URL is listening by the time the URL exists — the alternative is a race dressed up as a test.
/// Whatever `authorization` header arrives inside the TLS session is sent back through `heard`.
// The `Err` half of the upgrade callback's result is `tungstenite::handshake::server::
// ErrorResponse`, whose size is that crate's to choose and not ours. Nothing here ever returns it.
#[allow(clippy::result_large_err)]
async fn tls_node(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    heard: mpsc::UnboundedSender<Option<String>>,
) -> String {
    let config = ServerConfig::builder_with_provider(server_provider())
        .with_safe_default_protocol_versions()
        .expect("the provider offers a protocol version")
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .expect("the leaf and its key agree");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("no loopback port");
    let address = listener.local_addr().expect("no local address");

    tokio::spawn(async move {
        let acceptor = TlsAcceptor::from(Arc::new(config));
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let heard = heard.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else { return };
                let websocket = tokio_tungstenite::accept_hdr_async(tls, |request: &_, response| {
                    let _ = heard.send(header(request));
                    Ok(response)
                })
                .await;
                let Ok(websocket) = websocket else { return };
                let node = Node::builder()
                    .name("tls-acceptor")
                    .kind(NodeKind::Service)
                    .capability(OverTlsServer(Echo))
                    .build()
                    .expect("a node");
                let Ok(_connection) = node
                    .accept(TlsServerTransport(websocket), AcceptOptions::default())
                    .await
                else {
                    return;
                };
                // Held, not dropped: dropping the connection closes it, and the call the test is
                // about has not been made yet.
                std::future::pending::<()>().await;
            });
        }
    });

    format!("wss://{address}/zyris/v1/ws")
}

fn header(request: &tokio_tungstenite::tungstenite::handshake::server::Request) -> Option<String> {
    let value = request.headers().get("authorization")?;
    value.to_str().ok().map(str::to_string)
}

// --- the test ---------------------------------------------------------------------------------

/// Everything is wrapped in a deadline. A handshake that never completes would otherwise hang the
/// suite, and a hung test binary in this workspace outlives the `cargo` that started it.
const PATIENCE: Duration = Duration::from_secs(10);

#[tokio::test]
async fn a_wss_dial_completes_a_real_handshake_and_speaks_zyris_inside_it() {
    let ours = TestCa::new("zyris test CA");
    let stranger = TestCa::new("a CA nobody trusts");

    // The only trust anchor in this process, for the whole of this test. `NamedTempFile` is held
    // in a binding rather than a temporary: `tokio-tungstenite` builds its client config on every
    // dial, so the file has to still be there at each one.
    let anchor = tempfile::NamedTempFile::new().expect("a temp file");
    std::fs::write(anchor.path(), ours.pem.as_bytes()).expect("the CA is written");
    std::env::set_var("SSL_CERT_FILE", anchor.path());

    let (heard_tx, mut heard_rx) = mpsc::unbounded_channel();
    let (chain, key) = ours.loopback_leaf();
    let url = tls_node(chain, key, heard_tx.clone()).await;

    let dialer = Node::builder().name("tls-dialer").kind(NodeKind::Cli).build().expect("a node");

    let connection = tokio::time::timeout(PATIENCE, dialer.dial(&url, "znt_probe"))
        .await
        .expect("the dial has to settle inside the deadline")
        .expect("a certificate signed by the only trusted CA has to be accepted");

    // The credential travelled inside the session, not beside it.
    let seen = tokio::time::timeout(PATIENCE, heard_rx.recv())
        .await
        .expect("the server saw the upgrade")
        .flatten();
    assert_eq!(
        seen.as_deref(),
        Some("Bearer znt_probe"),
        "the header `ws::connect` adds has to arrive, and it has to arrive decrypted"
    );

    // And Zyris is what is being spoken over it, not merely bytes.
    let over_tls: OverTlsClient =
        connection.wait_capability(PATIENCE).await.expect("the peer announced its capability");
    let answered = tokio::time::timeout(PATIENCE, over_tls.echo("hello".to_string()))
        .await
        .expect("the call has to settle inside the deadline")
        .expect("the call has to be answered");
    assert_eq!(answered, Pong { said: "hello".to_string() });

    // The same client, the same trust anchor, a certificate from a CA it has never heard of. If
    // this were accepted, everything above would be passing against a verifier that is not
    // verifying — which is the state a handshake test is most likely to be quietly wrong in.
    let (impostor_chain, impostor_key) = stranger.loopback_leaf();
    let impostor = tls_node(impostor_chain, impostor_key, heard_tx).await;
    let refused = tokio::time::timeout(PATIENCE, dialer.dial(&impostor, "znt_probe"))
        .await
        .expect("the dial has to settle inside the deadline")
        .err()
        .expect("a certificate from an untrusted CA is not a connection");
    assert!(
        matches!(refused, ConnectError::Unreachable(_)),
        "an unverifiable certificate is the network's answer, not a build fault: {refused:?}"
    );
    let said = refused.to_string();
    assert!(
        said.contains("UnknownIssuer") || said.contains("unknown issuer"),
        "the refusal has to say the chain is what failed, got {said:?}"
    );

    drop(anchor);
}

// What this does not prove, so that a green run is not read as more than it is:
//
//   * Chain validation against a real CA, or the platform store on any OS. `SSL_CERT_FILE`
//     *replaces* the native store, so `platform::load_native_certs()` — the code that actually
//     runs against `attacca.cc` — is never executed here.
//   * SNI. An IP-literal server name means rustls sends no SNI extension at all, so nothing is
//     said about a reverse proxy that routes on one.
//   * ALPN, TLS 1.2, revocation, resumption, or more than one connection.
//   * That `install_chosen_provider` is what made this work. In a build with exactly one provider
//     linked, `ClientConfig::builder()` would have reached
//     `get_default_or_install_from_crate_features()` and got the same answer on its own. The case
//     where the explicit install is load-bearing is *both* providers linked, where that function
//     returns `None` — and no invocation in CI compiles that combination either.
//   * Anything about `reqwest`, which enrolment uses. It is on `rustls-platform-verifier`, a
//     different verifier that does not read `SSL_CERT_FILE`, so the device-grant road into TLS is
//     as untested as it was.
