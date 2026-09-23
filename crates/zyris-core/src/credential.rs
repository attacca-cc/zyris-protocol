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
///
/// Only read by `enroll::device` (behind the `enroll` feature) and this module's own tests — gated
/// the same way so a plain build never carries a `never used` warning for it.
#[cfg(any(feature = "enroll", test))]
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Hand-written rather than derived: a `zc_` never expires, so a `Debug` printed to a log by
/// accident — `tracing::debug!(?credential)`, an `unwrap_err` in a test failure — must not be the
/// whole bearer. Every other field is worth seeing whole; only `secret` is redacted.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("version", &self.version)
            .field("secret", &redacted(&self.secret))
            .field("system", &self.system)
            .field("program", &self.program)
            .field("scopes", &self.scopes)
            .field("owner_email", &self.owner_email)
            .finish()
    }
}

/// The first three bytes plus an ellipsis — enough to recognise which credential this was
/// (`zc_` is the only prefix that ships) without printing anything a reader could dial with.
///
/// `pub(crate)` rather than private: `enroll::protocol::TokenResponse` carries the same raw
/// `zc_` before it becomes a `Credential`, and its hand-written `Debug` reuses this rather than
/// growing a second redaction rule that could drift from this one.
pub(crate) fn redacted(secret: &str) -> String {
    let prefix: String = secret.chars().take(3).collect();
    format!("{prefix}…")
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
        let error = serde_json::from_str::<Credential>(old).expect_err("the old shape must not parse");
        assert!(
            error.to_string().contains("missing field"),
            "has to fail because a field is missing, not for some other reason, got: {error}"
        );
    }

    /// A `zc_` never expires, so a `Debug` a caller logs by accident must not be the whole story —
    /// only enough to tell which credential it was.
    #[test]
    fn debug_redacts_the_secret_but_names_who_it_is() {
        let printed = format!("{:?}", laptop());
        assert!(
            !printed.contains("zc_kept_by_the_caller"),
            "the full secret must never land in a Debug print, got: {printed}"
        );
        assert!(printed.contains("zc_…"), "a truncated hint is fine, got: {printed}");
        assert!(printed.contains("Laptop"), "the system name has to survive, got: {printed}");
        assert!(printed.contains("zyris-code"), "the program name has to survive, got: {printed}");
    }
}
