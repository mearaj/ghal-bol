//! Process-wide rustls 0.23 crypto provider (required before TLS/WSS).

use std::sync::Once;

static INSTALL: Once = Once::new();

/// Install aws-lc-rs as the default rustls `CryptoProvider` once per process.
pub fn ensure_rustls_crypto_provider() {
    INSTALL.call_once(|| {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("rustls CryptoProvider install_default");
    });
}
