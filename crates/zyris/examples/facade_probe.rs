//! Every step of the flow, reached through `zyris` and nothing else.
//!
//! Nothing here names `zyris-core`, `zyris-caps` or `zyris-capkit`. Enrollment, the account layer,
//! node registration, dialing and all four error enums are `zyris::…` paths, so if one of them ever
//! stops being re-exported this file stops compiling and CI says so. `tokio` is the exception and
//! has to be: a crate cannot hand out an async runtime.
//!
//! It is also runnable, and worth running against a real deployment once:
//!
//! ```text
//! cargo run -p zyris --example facade_probe --features enroll
//! ```

use zyris::enroll::{EnrollRequest, Progress};
use zyris::{
    Account, AccountCredential, ConnectError, EnrollError, Link, Node, NodeKind, NodeSpec,
    NodeToken, RegisterError, RotateError,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = std::env::var("ZYRIS_SERVER_URL")
        .unwrap_or_else(|_| zyris::DEFAULT_SERVER_URL.to_string());

    // 1. The code comes back as a value. Printing it is this program's decision, and the only
    //    reason there is a `println!` anywhere in this flow.
    let mut enrollment = zyris::enroll(
        &server,
        EnrollRequest {
            name: "facade probe".to_string(),
            platform: std::env::consts::OS.to_string(),
            scopes: vec!["agents:read".to_string(), "nodes:write".to_string()],
        },
    )
    .await
    .map_err(say_enroll)?;

    let code = enrollment.code().clone();
    println!("open {} and enter {}", code.verification_uri, code.user_code);

    // `poll` honours the server's interval and RFC 8628's `slow_down` itself, so this loop does not
    // sleep. `Lapsed` is not renewed automatically: a loop that never ends belongs to a program.
    let credential: AccountCredential = loop {
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

    // 2. The account credential is the only thing that rotates. Where the rotation goes is the
    //    caller's business, and the library waits for this to return Ok before using the new token.
    let account = Account::restore(&server, credential)
        .on_rotate(|rotated: AccountCredential| async move {
            println!("rotated; node {} has a fresh pair", rotated.node_id);
            Ok::<(), RotateError>(())
        })
        .build();

    let _bearer: String = account.bearer().await.map_err(say_enroll)?;

    // 3. One credential, as many nodes as you want.
    let token: NodeToken = account
        .register_node(NodeSpec {
            name: "facade probe node".to_string(),
            platform: Some(std::env::consts::OS.to_string()),
            scopes: vec!["agents:read".to_string()],
        })
        .await
        .map_err(say_register)?;
    println!("registered {} as {}", token.node_id, token.slug);

    // 4. Connecting takes a bearer string, so a node holding a `znt_` out of a secret manager never
    //    has to know the account layer exists.
    let link: Link = Node::builder()
        .name("facade probe node")
        .kind(NodeKind::Cli)
        .build()?
        .connect(&server, &token)
        .await
        .map_err(say_connect)?;

    println!("connected as {}", link.node_id());
    link.disconnect().await;
    Ok(())
}

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
        // The distinction the daemon needed an AtomicBool for: a person has to act.
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
    }
}
