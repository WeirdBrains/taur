//! The ACME client must build when rustls has both crypto providers linked.
//!
//! This file is its own test process, so nothing has called
//! `CryptoProvider::install_default` before `InstantAcme::issue`.

use std::time::Duration;
use taur_server::acme::{AcmeSettings, CertificateIssuer, ChallengeBoard, InstantAcme};

#[tokio::test]
async fn acme_client_does_not_panic_with_both_providers() {
    let dir = tempfile::tempdir().unwrap();
    let settings = AcmeSettings {
        domains: vec!["pkg.example.com".into()],
        directory: "https://127.0.0.1:9/directory".into(),
        staging: true,
        contact: "mailto:admin@example.com".into(),
        accept_terms: true,
        cache: dir.path().to_path_buf(),
        http_bind: "127.0.0.1:0".parse().unwrap(),
        renew_before: Duration::from_secs(30 * 24 * 3600),
    };
    let err = InstantAcme
        .issue(&settings, &ChallengeBoard::default())
        .await
        .expect_err("port 9 is not an ACME directory");
    let lower = err.to_ascii_lowercase();
    assert!(
        !lower.contains("cryptoprovider") && !lower.contains("aws-lc-rs"),
        "ACME client panicked choosing a rustls provider: {err}"
    );
    assert!(
        lower.contains("connect")
            || lower.contains("refused")
            || lower.contains("reset")
            || lower.contains("handshake")
            || lower.contains("tls"),
        "expected a local directory failure, got: {err}"
    );
}
