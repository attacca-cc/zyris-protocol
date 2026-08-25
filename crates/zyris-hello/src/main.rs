//! A minimal, complete Zyris node. Copy this crate as the starting point for your own.
//!
//! It does the two things every node does: it *announces* a capability (one `greet` tool, callable
//! by any agent the owning user runs) and it *consumes* the `attacca_api` capability the server
//! announces back — both over the same websocket, which is the point of the protocol.
//!
//! It is longer than it used to be, on purpose. `zyris` is a library: it does not read your
//! environment, print your enrollment code, choose where your credential lives, or decide when your
//! process ends. Everything below that is not the greeter is one of those four decisions, made once
//! here so you can see the shape and then make it differently.

mod greeter;
#[cfg(feature = "transfer")]
mod transfer;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use greeter::{HelloServer, RandomGreeter};
use zyris_attacca::{AttaccaApi, AttaccaApiClient};
use zyris::caps::TerminalServer;
use zyris::enroll::{EnrollRequest, Progress};
use zyris::{
    Account, AccountCredential, Connection, ErrorCode, Node, NodeKind, NodeSpec, NodeToken,
    RotateError,
};
use zyris_capkit::PtyTerminal;

#[cfg(feature = "desktop")]
use zyris::caps::{ImageFormat, InputServer, ScreenCaptureServer};
#[cfg(feature = "desktop")]
use zyris::NodeBuilder;
#[cfg(feature = "desktop")]
use zyris_capkit::{EnigoInput, HostDisplays, HostScreenCapture};

/// The server announces `attacca_api` immediately after the handshake; this is generous headroom.
pub(crate) const CONSUME_WAIT: Duration = Duration::from_secs(5);

/// What the **account** grant has to carry. `nodes:write` is the one that matters: without it
/// `register_node` comes back `Forbidden` and this node has no token to dial with.
const ACCOUNT_SCOPES: &[&str] = &["agents:read", "nodes:write"];

/// What the **node token** carries, which is deliberately less. A static node token must never be
/// able to mint another one, so `nodes:write` stops at the account layer.
///
/// `peers:write` is what lets a node publish its own peer address and look up another one on the
/// same account. Without it `peer_publish` comes back `ForbiddenScope`, nothing is ever published,
/// and `peer_lookup` has nothing to answer with — so no peer can find this node and no transfer
/// can start. Asked for only when the feature is on, because a node built without transfer has no
/// use for it.
#[cfg(feature = "transfer")]
const NODE_SCOPES: &[&str] = &["agents:read", "peers:write"];
#[cfg(not(feature = "transfer"))]
const NODE_SCOPES: &[&str] = &["agents:read"];

#[tokio::main]
async fn main() -> ExitCode {
    // Before anything opens a TLS connection, and before the logger, because the panic this avoids
    // takes the whole node down on the first connect and leaves the reason in a backtrace rather
    // than in the log. `zyris::p2p::tls` explains what the two providers are and why neither this
    // node nor its configuration can settle it.
    #[cfg(feature = "transfer")]
    zyris::p2p::tls::install_default_provider();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "zyris_hello=info,zyris=info".into()),
        )
        .init();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "this node stopped");
            ExitCode::from(1)
        }
    }
}

/// The whole flow, in the order it happens: a credential, an account, a node token, a connection.
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let server = std::env::var("ZYRIS_SERVER_URL")
        .unwrap_or_else(|_| zyris::DEFAULT_SERVER_URL.to_string());
    let node_name = std::env::var("ZYRIS_NODE_NAME")
        .ok()
        .or_else(zyris::machine_name)
        .unwrap_or_else(|| "zyris-hello".to_string());

    let credential = match read_credential()? {
        Some(credential) => credential,
        None => authorize(&server, &node_name).await?,
    };

    // The library waits for `on_rotate` to return `Ok` before it uses a rotated pair. That is not
    // politeness: a refresh token is single-use, and a crash between "used" and "saved" is how a
    // node gets revoked outright.
    let account = Account::restore(&server, credential)
        .on_rotate(|rotated: AccountCredential| async move {
            write_credential(&rotated).map_err(|error| RotateError(error.to_string()))
        })
        .build();

    // One account credential, as many node tokens as you like — a second window, a second working
    // directory, a second identity in the node list. This node keeps exactly one, and that is the
    // point: a `znt_` never expires and never rotates, so the token *is* this node's identity
    // across restarts. Minting one per launch would put a fresh node in the account every time the
    // process starts, and the per-user cap is real.
    let token = match read_node_token()? {
        Some(saved) => {
            tracing::info!(node_id = %saved.node_id, slug = %saved.slug, "reusing this node");
            saved
        }
        None => {
            let minted = account
                .register_node(NodeSpec {
                    name: node_name.clone(),
                    platform: Some(std::env::consts::OS.to_string()),
                    scopes: NODE_SCOPES.iter().map(|scope| scope.to_string()).collect(),
                })
                .await?;
            write_node_token(&minted)?;
            tracing::info!(node_id = %minted.node_id, slug = %minted.slug, "registered this node");
            minted
        }
    };

    let builder = Node::builder()
        .name(node_name.as_str())
        .kind(NodeKind::Service)
        .capability(HelloServer(RandomGreeter::new(node_name.as_str())))
        .capability(TerminalServer(PtyTerminal::default()));

    #[cfg(feature = "desktop")]
    let builder = with_desktop(builder);

    // Built before the dial, because a node announces what it can do before it has anywhere to say
    // it. The rendezvous client and the accept loop both arrive later, on the connection.
    #[cfg(feature = "transfer")]
    let transfer = match transfer::Transfer::bind(node_name.as_str()).await {
        Ok(transfer) => Some(std::sync::Arc::new(transfer)),
        Err(error) => {
            tracing::warn!(%error, "could not bind the peer endpoint; file transfer is off");
            None
        }
    };
    #[cfg(feature = "transfer")]
    let builder = match &transfer {
        Some(transfer) => builder.capability(transfer.capability()),
        None => builder,
    };

    let node = builder
        .on_connect(move |conn| {
            #[cfg(feature = "transfer")]
            let transfer = transfer.clone();
            async move {
                report_server_capabilities(&conn).await;
                #[cfg(feature = "transfer")]
                if let Some(transfer) = transfer {
                    transfer.on_connect(&conn).await;
                }
            }
        })
        .build()?;

    let link = node.connect(&server, &token).await?;
    tracing::info!(node_id = link.node_id(), "connected");

    // Ctrl-C is this program's, not the library's. `wait_closed` resolves on its own if the link
    // dies for good.
    tokio::select! {
        result = link.wait_closed() => result?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("stopping");
            link.disconnect().await;
        }
    }
    Ok(())
}

/// Where this node keeps its account credential between runs.
///
/// **The library does not choose this.** Replace the whole function to put it in a keychain, a k8s
/// Secret, or a row in your own database — the credential is a plain serializable value.
fn credential_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("ZYRIS_HELLO_CREDENTIAL") {
        return PathBuf::from(explicit);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("zyris-hello").join("credential.json")
}

fn read_credential() -> Result<Option<AccountCredential>, Box<dyn std::error::Error>> {
    match std::fs::read_to_string(credential_path()) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_credential(credential: &AccountCredential) -> std::io::Result<()> {
    let path = credential_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(credential).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    // A refresh token in a world-readable file is the failure this line exists for.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Where this node's own token lives, beside the account credential and under the same rule.
///
/// **The library does not choose this either.** `NodeToken` is a plain serializable value for
/// exactly that reason — the crate hands it over once, keeps no copy, and has no opinion about
/// whether it ends up in a file, a keychain or a database row.
fn node_token_path() -> PathBuf {
    if let Some(explicit) = std::env::var_os("ZYRIS_HELLO_NODE_TOKEN") {
        return PathBuf::from(explicit);
    }
    credential_path().with_file_name("node-token.json")
}

fn read_node_token() -> Result<Option<NodeToken>, Box<dyn std::error::Error>> {
    match std::fs::read_to_string(node_token_path()) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_node_token(token: &NodeToken) -> std::io::Result<()> {
    let path = node_token_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(token).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    // A `znt_` never expires, so a world-readable copy of one is worse than a leaked access token.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The enrollment half, the one time a person is involved.
///
/// The code arrives as a value; printing it is this program's decision, which is why the block
/// below is here and not in `zyris`. A node with a screen draws it instead.
async fn authorize(
    server: &str,
    node_name: &str,
) -> Result<AccountCredential, Box<dyn std::error::Error>> {
    let mut enrollment = zyris::enroll(
        server,
        EnrollRequest {
            name: node_name.to_string(),
            platform: std::env::consts::OS.to_string(),
            scopes: ACCOUNT_SCOPES.iter().map(|scope| scope.to_string()).collect(),
        },
    )
    .await?;

    let code = enrollment.code().clone();
    println!(
        "\n\
         --------------------------------------------------------------\n  \
         Authorize this node\n\n  \
         1. Open        {uri}\n  \
         2. Enter code  {user_code}\n\n  \
         Waiting for approval. Press Ctrl-C to cancel.\n\
         --------------------------------------------------------------\n",
        uri = code.verification_uri,
        user_code = code.user_code,
    );

    // `poll` keeps the server's interval and RFC 8628's `slow_down` itself, so this loop does not
    // sleep. A lapsed code is renewed here rather than in the library: an endless retry loop is a
    // policy, and policies belong to programs.
    loop {
        match enrollment.poll().await? {
            Progress::Waiting { remaining } => {
                tracing::debug!(seconds_left = remaining.as_secs(), "waiting for approval");
            }
            Progress::Granted(credential) => {
                write_credential(&credential)?;
                tracing::info!(account = %credential.owner_email, "authorized");
                return Ok(credential);
            }
            Progress::Lapsed => {
                enrollment.renew().await?;
                println!("  That code expired. New code: {}", enrollment.code().user_code);
            }
            Progress::Denied => return Err("the request was declined".into()),
        }
    }
}

/// Announce `screen_capture` and `input` — the two capabilities that need a display to exist.
///
/// This node sends screenshots at the display's own resolution: no default `max_width`, and
/// [`HostScreenCapture::without_budget`] to switch off the pass that re-encodes smaller until the
/// bytes fit inline. A scaled capture makes image coordinates stop being display coordinates, and
/// the model then has to read a multiplier out of the image's description before `input.move_to`
/// will land where it meant. At 1:1 there is nothing to apply. A caller that wants a smaller image
/// can still pass `max_width` per call.
///
/// `input` is announced only if the display server actually accepts a connection. A node that
/// cannot type should not offer to: announcing it and failing every call is worse than never
/// appearing in the tool list, because an agent has no way to tell the two apart.
#[cfg(feature = "desktop")]
fn with_desktop(builder: NodeBuilder) -> NodeBuilder {
    let screen = HostScreenCapture::default().with_format(ImageFormat::Jpeg).without_budget();
    let backend = screen.backend();
    tracing::info!(backend = ?backend, "announcing screen_capture");
    let builder = builder.capability(ScreenCaptureServer(screen));

    // The same backend the screenshots come from: `move_to` takes display-local coordinates and
    // adds that display's origin, so it has to agree with `screenshot` about where a monitor is.
    match EnigoInput::new(HostDisplays(backend)) {
        Ok(input) => {
            tracing::info!("announcing input");
            builder.capability(InputServer(input))
        }
        Err(error) => {
            tracing::warn!(%error, "no display server for input; not announcing it");
            builder
        }
    }
}

/// The consume half. A node is not only a tool provider: the server announces `attacca_api` on the
/// same connection, so this process can drive agents and sessions while serving `greet`. Failures
/// here are logged and ignored — a node whose token lacks `agents:read` should still serve tools.
///
/// The client is `zyris-attacca`'s, the crate that declares `attacca_api` — the consumer side of a
/// capability is a trait like any other, and importing one someone already wrote is the short path.
/// When you consume a capability nobody has published, declare the slice you call yourself: one
/// `#[zyris::capability]` trait naming the methods you use resolves against the real announcement,
/// because matching is by `(name, version)` and the announced tool list is never compared.
async fn report_server_capabilities(conn: &Connection) {
    match conn.wait_capability::<AttaccaApiClient>(CONSUME_WAIT).await {
        Ok(api) => {
            tracing::info!("server announced attacca_api; this node can call back into Attacca");
            match api.list_agents().await {
                Ok(agents) => tracing::info!(
                    count = agents.len(),
                    first = agents.first().map(|a| a.name.as_str()).unwrap_or("-"),
                    "attacca_api.list_agents ok"
                ),
                Err(e) if e.code == ErrorCode::ForbiddenScope => {
                    tracing::info!("this node's token has no agents:read scope; skipping the call")
                }
                Err(e) => tracing::warn!(error = %e, "attacca_api.list_agents failed"),
            }
        }
        Err(e) => tracing::warn!(error = %e, "server did not announce attacca_api"),
    }
}
