//! The whole of Zyris in one file: announce a capability, and answer "Hello World" when a model
//! calls it.
//!
//! Everything here is a `zyris::…` path. Nothing names `zyris-core`, `zyris-caps` or any other
//! crate in the repository, so if a re-export ever moves this example stops compiling and CI says
//! so. `tokio` is the exception and has to be: a crate cannot hand out an async runtime.
//!
//! ```text
//! # the short path — you already hold a node token
//! ZYRIS_NODE_TOKEN=znt_… cargo run -p zyris --example hello --features enroll
//!
//! # the long path — no credential yet. Prints an 8-character code to approve in a browser.
//! cargo run -p zyris --example hello --features enroll
//! ```
//!
//! Once it says `serving`, ask an agent on that account to call `hello.greet`.
//!
//! # The four things this file is
//!
//! 1. **A capability.** One trait, one method, one derive-able answer. That is the whole surface a
//!    node offers.
//! 2. **A way to get a token.** Either read one, or enrol and register a node — the second is the
//!    part with the 8-character code in it.
//! 3. **A connection**, which serves the capability for as long as it is up.
//! 4. **Errors worth telling apart.** Every enum is matched exhaustively rather than printed, so
//!    adding a variant upstream breaks this file instead of silently falling into a catch-all.

use zyris::enroll::{EnrollRequest, Progress};
use zyris::schemars::JsonSchema;
use zyris::serde::{Deserialize, Serialize};
use zyris::{
    Account, AccountCredential, ConnectError, EnrollError, Link, Node, NodeKind, NodeSpec,
    NodeToken, RegisterError, RotateError,
};

// ---------------------------------------------------------------------------------------------
// 1. The capability
// ---------------------------------------------------------------------------------------------

/// What `hello.greet` hands back.
///
/// Every field is agent-visible: schemars turns these doc comments into the JSON-Schema field
/// descriptions a model reads before it decides what to do with the answer. That is the same
/// mechanism that carries the trait's doc comment out as the tool description, so prose here is
/// part of the wire rather than a note to the next reader.
///
/// The two `crate = …` attributes point the derives at this crate's own re-exports, so an example
/// living inside the face needs no `serde` or `schemars` dependency of its own. A program with
/// those crates in its `Cargo.toml` writes the derives plainly and drops both lines.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "zyris::serde")]
#[schemars(crate = "zyris::schemars")]
pub struct Greeting {
    /// The greeting itself.
    pub message: String,
    /// Which node answered, so an account with several of them can tell which one spoke.
    pub node: String,
}

/// A node that says hello.
///
/// `#[zyris::capability]` turns this one declaration into five things: the trait itself (async,
/// `Send + Sync + 'static`), a request struct per method, a `hello_capability()` descriptor,
/// a `HelloServer<T>` to hand to `Node::builder().capability(..)`, and a `HelloClient` for anyone
/// calling the other way. The name and version here are what the peer sees.
#[zyris::capability(name = "hello", version = 1)]
pub trait Hello {
    /// Say hello, optionally to someone in particular.
    async fn greet(&self, name: Option<String>) -> zyris::Result<Greeting>;
}

/// The implementation. It is an ordinary struct with an ordinary async method — the protocol does
/// not ask it to be anything else.
struct HelloWorld {
    node: String,
}

#[zyris::async_trait]
impl Hello for HelloWorld {
    async fn greet(&self, name: Option<String>) -> zyris::Result<Greeting> {
        let message = match name.as_deref().map(str::trim).filter(|who| !who.is_empty()) {
            Some(who) => format!("Hello, {who}!"),
            None => "Hello World".to_string(),
        };
        Ok(Greeting { message, node: self.node.clone() })
    }
}

// ---------------------------------------------------------------------------------------------
// 2–4. Getting a token, connecting, and staying up
// ---------------------------------------------------------------------------------------------

const NODE_NAME: &str = "hello";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server =
        std::env::var("ZYRIS_SERVER_URL").unwrap_or_else(|_| zyris::DEFAULT_SERVER_URL.to_string());

    // A node connects with a bearer string and nothing else. Where it came from — a secret
    // manager, a file, the enrollment below — is the program's business, never the library's.
    let token = match std::env::var("ZYRIS_NODE_TOKEN") {
        Ok(token) if !token.trim().is_empty() => token,
        _ => take_a_node(&server).await?,
    };

    let link: Link = Node::builder()
        .name(NODE_NAME)
        .kind(NodeKind::Service)
        .capability(HelloServer(HelloWorld { node: NODE_NAME.to_string() }))
        .build()?
        .connect(&server, &token)
        .await
        .map_err(say_connect)?;

    println!("serving hello.greet as {}; Ctrl-C to stop", link.node_id());

    // `connect` keeps the link up across drops, so the only two ways out are the server giving up
    // on us and a person asking us to stop. Which of those ends the program is this program's
    // decision — the library has no opinion and no signal handler.
    tokio::select! {
        closed = link.wait_closed() => {
            closed.map_err(say_connect)?;
            println!("the server closed the link");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("stopping");
            link.disconnect().await;
        }
    }
    Ok(())
}

/// The long path: no credential yet, so ask for one, then take a node under it.
///
/// This is the only part of the file that prints anything a person has to act on, and it prints it
/// because *this program* decided to. `enroll` hands the code back as a value; a library that
/// printed it would put it underneath a TUI's own screen, or in a journal nobody reads.
async fn take_a_node(server: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut enrollment = zyris::enroll(
        server,
        EnrollRequest {
            name: NODE_NAME.to_string(),
            platform: std::env::consts::OS.to_string(),
            // `nodes:write` is what lets the credential take a node below. Ask for nothing else:
            // the scopes approved here are the ceiling for every node this credential ever takes.
            scopes: vec!["nodes:write".to_string()],
        },
    )
    .await
    .map_err(say_enroll)?;

    println!("open {} and enter {}", enrollment.code().verification_uri, enrollment.code().user_code);

    // `poll` honours the server's interval and RFC 8628's `slow_down` itself, so this loop does no
    // sleeping of its own. `Lapsed` is not renewed automatically: a loop that never ends belongs
    // to a program, not to a library.
    let credential: AccountCredential = loop {
        match enrollment.poll().await.map_err(say_enroll)? {
            Progress::Waiting { .. } => {}
            Progress::Granted(credential) => break credential,
            Progress::Lapsed => {
                enrollment.renew().await.map_err(say_enroll)?;
                println!("that code expired; the new one is {}", enrollment.code().user_code);
            }
            Progress::Denied => return Err("the request was declined".into()),
        }
    };

    // The account credential is the one thing that rotates. Where the new pair goes is the
    // caller's business, and the library will not use it until this returns `Ok` — so a program
    // that writes it to disk cannot end up with a token the server has already retired.
    let account = Account::restore(server, credential)
        .on_rotate(|rotated: AccountCredential| async move {
            // A real program stores `rotated` here. Losing it costs a re-enrollment.
            let _ = rotated;
            Ok::<(), RotateError>(())
        })
        .build();

    let node: NodeToken = account
        .register_node(NodeSpec {
            name: NODE_NAME.to_string(),
            platform: Some(std::env::consts::OS.to_string()),
            scopes: vec![],
        })
        .await
        .map_err(say_register)?;

    // Read the slug back rather than assuming the name asked for. Two machines volunteering the
    // same name is ordinary, so the server may hand back one it disambiguated — `hello`, then
    // `hello-1` — and the slug is what a tool call is routed by.
    println!("took node {} as {}", node.node_id, node.slug);
    println!("set ZYRIS_NODE_TOKEN to skip this next time");
    Ok(node.token)
}

// ---------------------------------------------------------------------------------------------
// 4. The errors, each matched rather than printed
// ---------------------------------------------------------------------------------------------

fn say_enroll(error: EnrollError) -> String {
    match error {
        EnrollError::Denied => "the request was declined".to_string(),
        EnrollError::Lapsed => "the code expired".to_string(),
        // The one answer no retry and no fresh code can fix: the grant chain itself is dead.
        EnrollError::Revoked => "revoked; authorize this node again".to_string(),
        EnrollError::ScopeUnknown { scope } => {
            format!("this deployment does not know the scope {scope}")
        }
        EnrollError::Unreachable(transport) => format!("worth retrying: {transport}"),
    }
}

fn say_register(error: RegisterError) -> String {
    match error {
        RegisterError::Forbidden => "this credential lacks nodes:write".to_string(),
        RegisterError::ScopeExceeded { requested, granted } => {
            format!("asked for {requested:?}, the account grants {granted:?}")
        }
        RegisterError::Revoked => "the account credential is dead; authorize it again".to_string(),
        RegisterError::Unreachable(transport) => format!("worth retrying: {transport}"),
    }
}

fn say_connect(error: ConnectError) -> String {
    match error {
        // A person has to act on these two, and no amount of retrying substitutes.
        ConnectError::Revoked => "revoked; authorize this node again".to_string(),
        ConnectError::Unauthorized => "the server did not accept this token".to_string(),
        // A 426 refuses the upgrade before either side has spoken the protocol, so there is
        // nothing on the wire to name the server's version; only a `HelloAck` mismatch fills it in.
        ConnectError::VersionMismatch { ours, theirs } => {
            let theirs = theirs.unwrap_or_else(|| "an unnamed version".to_string());
            format!("we speak {ours}, it speaks {theirs}; retrying will not help")
        }
        // …and the one it must not be confused with: retrying is the whole answer.
        ConnectError::Unreachable(transport) => format!("worth retrying: {transport}"),
        // A build fault, not a network one: no `tls-ring` or `tls-aws-lc`, so rustls has
        // nothing to negotiate `wss://` with. Retrying reaches the same wall every time.
        ConnectError::NoTlsProvider => "this build named no TLS provider".to_string(),
    }
}
