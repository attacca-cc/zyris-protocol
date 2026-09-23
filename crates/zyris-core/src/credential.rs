//! What an enrollment issues, and what the caller stores: one `zc_` credential for one (system,
//! program) pair.
//!
//! It does not expire and it does not rotate, so it is written down once and read back on every
//! start. Where it lives between runs is the caller's business. This crate never picks a path: a
//! laptop wants a file under `$HOME`, a pod wants a Secret, a desktop app wants the OS keychain,
//! and a test wants nothing at all.
//!
//! A credential exists to create nodes: every connection made with it is a node, named by
//! `Node::builder().name(..)`, and the server answers with where it put it (`Link::address`).

use serde::{Deserialize, Serialize};

/// Bumped only on an incompatible change. `1` was the `zna_`/`znr_` account credential, which no
/// server honours any more.
pub(crate) const CREDENTIAL_VERSION: u32 = 2;

/// Something the server keeps a name for: its id, the name a person gave it, and the slug that
/// name became in node paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named {
    pub id: String,
    pub name: String,
    pub slug: String,
}

/// A long-lived `zc_` bearer, issued to one program on one system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub version: u32,
    /// `zc_…`. What `Node::connect` presents.
    pub secret: String,
    /// The machine, as the approving person picked or created it.
    pub system: Named,
    /// The program this credential was issued to. Its `id` is the credential's own.
    pub program: Named,
    /// Every node made with this credential has exactly these.
    pub scopes: Vec<String>,
    /// The account it was issued under. Worth printing once: over SSH, against a URL typed from
    /// memory, it is the only chance to notice the wrong deployment.
    #[serde(default)]
    pub owner_email: String,
}

impl Credential {
    /// The bearer to dial with.
    pub fn secret(&self) -> &str {
        &self.secret
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> Credential {
        Credential {
            version: CREDENTIAL_VERSION,
            secret: "zc_kept_by_the_caller".into(),
            system: Named { id: "sys-1".into(), name: "Laptop".into(), slug: "laptop".into() },
            program: Named { id: "cred-1".into(), name: "zyris-code".into(), slug: "zyris-code".into() },
            scopes: vec!["agents:read".into()],
            owner_email: "allen@example.com".into(),
        }
    }

    /// The caller is the only place a credential lives, so what comes back from storage has to be
    /// the credential that went in.
    #[test]
    fn a_credential_can_be_written_down_and_read_back() {
        let written = serde_json::to_string(&laptop()).expect("a credential serializes");
        let read_back: Credential = serde_json::from_str(&written).expect("and reads back");
        assert_eq!(read_back, laptop());
        assert_eq!(read_back.secret(), "zc_kept_by_the_caller");
    }

    /// A file the old account layer wrote has no `secret`, `system` or `program`. Reading it has to
    /// fail loudly — not produce a credential with an empty bearer that dials and gets a 401 with no
    /// hint why.
    #[test]
    fn an_account_credential_from_before_is_not_read_as_one() {
        let old = r#"{"version":1,"access_token":"zna_old","refresh_token":"znr_old",
            "node_id":"n","node_name":"laptop","owner_email":"a@example.com","access_expires_at":1}"#;
        assert!(serde_json::from_str::<Credential>(old).is_err());
    }
}
