//! The whole of Zyris in one file: announce a capability, and answer "Hello World" when a model
//! calls it.
//!
//! Everything here is a `zyris::…` path. Nothing names `zyris-core`, `zyris-caps` or any other
//! crate in the repository, so if a re-export ever moves this example stops compiling and CI says
//! so. `tokio` is the exception and has to be: a crate cannot hand out an async runtime.
//!
//! ```text
//! # the short path — you already hold a credential
//! ZYRIS_CREDENTIAL=zc_… cargo run -p zyris --example hello --features enroll
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
//! 2. **A way to get a credential.** Either read one, or enrol — the second is the part with the
//!    8-character code in it.
//! 3. **A connection**, which is a node for as long as it is up.
//! 4. **Errors worth telling apart.** Every enum is matched exhaustively rather than printed, so
//!    adding a variant upstream breaks this file instead of silently falling into a catch-all.

use zyris::enroll::{EnrollRequest, Progress};
use zyris::schemars::JsonSchema;
use zyris::serde::{Deserialize, Serialize};
use zyris::{ConnectError, Credential, EnrollError, Link, Node, NodeKind};

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
// 2–4. Getting a credential, connecting, and staying up
// ---------------------------------------------------------------------------------------------

/// What this program calls itself when it enrols. It is fixed on the credential.
const PROGRAM: &str = "hello";
/// What each connection asks to be called. A second one live at the same `system/program` path —
/// even under a different credential named `hello` — is told `hello-2`.
const NODE_NAME: &str = "hello";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server =
        std::env::var("ZYRIS_SERVER_URL").unwrap_or_else(|_| zyris::DEFAULT_SERVER_URL.to_string());

    // A node connects with a bearer string and nothing else. Where it came from — a secret
    // manager, a file, the enrollment below — is the program's business, never the library's.
    let secret = match std::env::var("ZYRIS_CREDENTIAL") {
        Ok(secret) if !secret.trim().is_empty() => secret,
        _ => take_a_credential(&server).await?.secret,
    };

    let link: Link = Node::builder()
        .name(NODE_NAME)
        .kind(NodeKind::Service)
        .capability(HelloServer(HelloWorld { node: NODE_NAME.to_string() }))
        .build()?
        .connect(&server, &secret)
        .await
        .map_err(say_connect)?;

    // Read the address back rather than assuming the name asked for: it is the path a tool call is
    // routed by, and it ends in `hello-2` whenever another `hello` is live at the same
    // `system/program` path, across every credential with that program name.
    let placed = link.address().map(|address| address.path()).unwrap_or_else(|| link.node_id());
    println!("serving hello.greet as {placed}; Ctrl-C to stop");

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

/// The long path: no credential yet, so ask for one.
///
/// This is the only part of the file that prints anything a person has to act on, and it prints it
/// because *this program* decided to. `enroll` hands the code back as a value; a library that
/// printed it would put it underneath a TUI's own screen, or in a journal nobody reads.
async fn take_a_credential(server: &str) -> Result<Credential, Box<dyn std::error::Error>> {
    let mut enrollment = zyris::enroll(
        server,
        EnrollRequest {
            program: PROGRAM.to_string(),
            // Lets the approval screen preselect this machine. A hint, never verified.
            system_hint: zyris::machine_name().unwrap_or_default(),
            platform: std::env::consts::OS.to_string(),
            // A node that only serves a tool calls nothing back, so it needs no scopes at all.
            scopes: vec![],
        },
    )
    .await
    .map_err(say_enroll)?;

    println!("open {} and enter {}", enrollment.code().verification_uri, enrollment.code().user_code);

    // `poll` honours the server's interval and RFC 8628's `slow_down` itself, so this loop does no
    // sleeping of its own. `Lapsed` is not renewed automatically: a loop that never ends belongs
    // to a program, not to a library.
    let credential: Credential = loop {
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

    // It never expires and never rotates, so a real program writes it down here — a file, a
    // keychain, a Secret — and reads it back on every start instead of enrolling again. This
    // example has nowhere of its own to write it, so the secret is printed once: the person who
    // just approved it in a browser is the one who can act on it, and without the value here the
    // instruction to "set ZYRIS_CREDENTIAL" is not something they could actually follow.
    println!(
        "issued for {}/{}: {}\n\
         store this; it never expires — revoke it in Attacca when done. Set ZYRIS_CREDENTIAL to \
         this value to skip enrolling next time.",
        credential.system.slug, credential.program.slug, credential.secret
    );
    Ok(credential)
}

// ---------------------------------------------------------------------------------------------
// 4. The errors, each matched rather than printed
// ---------------------------------------------------------------------------------------------

fn say_enroll(error: EnrollError) -> String {
    match error {
        EnrollError::Denied => "the request was declined".to_string(),
        EnrollError::Lapsed => "the code expired".to_string(),
        EnrollError::ScopeUnknown { scope } => {
            format!("this deployment does not know the scope {scope}")
        }
        EnrollError::Unreachable(transport) => format!("worth retrying: {transport}"),
    }
}

fn say_connect(error: ConnectError) -> String {
    match error {
        // A person has to act on these two, and no amount of retrying substitutes. `Unauthorized`
        // is also what a revoked or retired credential reads as: forget it and enrol again.
        ConnectError::Revoked => "revoked; authorize this node again".to_string(),
        ConnectError::Unauthorized => "the server did not accept this credential".to_string(),
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
