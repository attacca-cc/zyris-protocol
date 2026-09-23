//! Every step of the flow, reached through `zyris` and nothing else.
//!
//! Nothing here names `zyris-core`, `zyris-caps` or `zyris-capkit`. Enrollment, the credential,
//! dialing, the address the server assigns and both error enums are `zyris::…` paths, so if one of
//! them ever stops being re-exported this file stops compiling and CI says so. `tokio` is the
//! exception and has to be: a crate cannot hand out an async runtime.
//!
//! It is also runnable, and worth running against a real deployment once:
//!
//! ```text
//! cargo run -p zyris --example facade_probe --features enroll
//! ```

use zyris::enroll::{EnrollRequest, Progress};
use zyris::{ConnectError, Credential, EnrollError, Link, Named, Node, NodeAddress, NodeKind};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = std::env::var("ZYRIS_SERVER_URL")
        .unwrap_or_else(|_| zyris::DEFAULT_SERVER_URL.to_string());

    // 1. The code comes back as a value. Printing it is this program's decision, and the only
    //    reason there is a `println!` anywhere in this flow.
    let mut enrollment = zyris::enroll(
        &server,
        EnrollRequest {
            program: "facade-probe".to_string(),
            system_hint: zyris::machine_name().unwrap_or_default(),
            platform: std::env::consts::OS.to_string(),
            scopes: vec!["agents:read".to_string()],
        },
    )
    .await
    .map_err(say_enroll)?;

    let code = enrollment.code().clone();
    println!("open {} and enter {}", code.verification_uri, code.user_code);

    // `poll` honours the server's interval and RFC 8628's `slow_down` itself, so this loop does not
    // sleep. `Lapsed` is not renewed automatically: a loop that never ends belongs to a program.
    let credential: Credential = loop {
        match enrollment.poll().await.map_err(say_enroll)? {
            Progress::Waiting { remaining } => {
                println!("waiting, {}s left", remaining.as_secs());
            }
            Progress::Granted(credential) => break credential,
            Progress::Lapsed => {
                enrollment.renew().await.map_err(say_enroll)?;
                println!("that code expired; the new one is {}", enrollment.code().user_code);
            }
            Progress::Denied => return Err("the request was declined".into()),
        }
    };

    // 2. The credential is the whole of what enrollment hands back, and all of it is serde.
    let Named { slug: system, .. } = &credential.system;
    println!("issued to {system}/{} under {}", credential.program.slug, credential.owner_email);

    // 3. Connecting takes a bearer string, so a node holding a `zc_` out of a secret manager never
    //    has to know enrollment exists.
    let link: Link = Node::builder()
        .name("facade probe node")
        .kind(NodeKind::Service)
        .build()?
        .connect(&server, credential.secret())
        .await
        .map_err(say_connect)?;

    // 4. Where the server put it.
    let placed: Option<NodeAddress> = link.address();
    println!(
        "connected as {} ({})",
        placed.map(|address| address.path()).unwrap_or_default(),
        link.node_id()
    );
    link.disconnect().await;
    Ok(())
}

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
        // The distinction the daemon needed an AtomicBool for: a person has to act.
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
