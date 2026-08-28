//! Which TLS provider this build uses, decided in one place.
//!
//! Rustls ships no cipher suite of its own. It reads its crate features and, when they name none,
//! `CryptoProvider::get_default_or_install_from_crate_features()` panics — deferred all the way to
//! the first connection, because there is no `compile_error!` anywhere in rustls for it. So a build
//! with no provider is green, starts, announces, and dies the moment it reaches the network.
//!
//! Two consumers here reach TLS by different roads and both would hit that:
//!
//! - `tokio-tungstenite` builds a client config inside `connect_async`, so a `wss://` dial panics.
//! - `reqwest`, on the `rustls-no-provider` feature, panics while *building the client* — before a
//!   request, before a URL, and regardless of scheme. That feature means "the application installs
//!   it", and this module is the application saying so.
//!
//! Installing rather than leaving rustls to read its own features is also what makes the answer
//! deterministic: `from_crate_features()` returns `None` when *both* providers are linked, which is
//! a panic for the ambiguity rather than for the absence, and a dependency graph can arrive at both
//! without anyone choosing it.

/// Put this build's chosen provider in place, once.
///
/// `install_default` reports an error rather than panicking once something is already installed,
/// and an application that installed its own is exactly who should win — so the result is dropped.
#[cfg(feature = "tls-aws-lc")]
pub(crate) fn install_chosen_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(all(feature = "tls-ring", not(feature = "tls-aws-lc")))]
pub(crate) fn install_chosen_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(not(any(feature = "tls-ring", feature = "tls-aws-lc")))]
pub(crate) fn install_chosen_provider() {}

/// Whether this build can do TLS at all.
///
/// Asked before a socket is opened or a client is built, so the answer is about the build and never
/// about the network.
///
/// One case answers `false` while rustls would in fact have coped: another crate in the graph
/// switched on `rustls/ring` or `rustls/aws-lc-rs`, nothing installed a default, and rustls would
/// have read that feature at the last moment. There is no way to ask it — the function that reads
/// those features is private and the one that is public panics. Refusing is the side to err on: the
/// message names both features and `install_default`, and either is one line, whereas the other
/// direction is a panic in a stranger's process.
#[cfg(feature = "client")]
pub(crate) fn provider_is_missing() -> bool {
    install_chosen_provider();
    rustls::crypto::CryptoProvider::get_default().is_none()
}

/// The same question for a URL that may not need an answer. `ws://` carries no TLS, so a missing
/// provider is not what stops it — refusing it would break every plaintext deployment for a reason
/// that does not apply to it.
#[cfg(feature = "client")]
pub(crate) fn missing_for(url: &str) -> bool {
    let scheme_is_tls = url.len() >= 4 && url[..4].eq_ignore_ascii_case("wss:");
    scheme_is_tls && provider_is_missing()
}
