//! Building an iroh endpoint that touches no public iroh infrastructure.

/// An endpoint that uses no public iroh infrastructure: `presets::Minimal` (no n0 address lookup,
/// no pkarr publishing), and a relay only if it is ours.
pub async fn private_endpoint(
    secret: iroh::SecretKey,
    relay: Option<iroh::RelayUrl>,
) -> Result<iroh::Endpoint, iroh::endpoint::BindError> {
    iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(secret)
        .alpns(vec![crate::transport::ALPN.to_vec()])
        .relay_mode(relay.map_or(iroh::RelayMode::Disabled, |url| iroh::RelayMode::custom([url])))
        .bind()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_private_endpoint_without_a_relay_binds_and_has_no_relay() {
        let ep = private_endpoint(iroh::SecretKey::generate(), None).await.unwrap();
        assert!(ep.addr().relay_urls().next().is_none());
        ep.close().await;
    }
}
