//! TLS configuration helpers built on rustls.

use anyhow::{Context, Result};
use rustls::{ClientConfig, RootCertStore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// Transport security mode for any of SMTP / IMAP / POP3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Plain TCP, no encryption (rare and discouraged).
    None,
    /// Connect in cleartext then upgrade with STARTTLS / STLS.
    #[serde(rename = "starttls")]
    StartTls,
    /// Implicit TLS on a dedicated port (465 SMTPS, 993 IMAPS, 995 POP3S).
    #[serde(rename = "ssl")]
    Implicit,
}

impl Security {
    pub fn as_str(self) -> &'static str {
        match self {
            Security::None => "none",
            Security::StartTls => "starttls",
            Security::Implicit => "ssl",
        }
    }
}

/// Build a rustls `ClientConfig` using the bundled Mozilla root store, plus
/// any extra CA bundle the user specified.  When `insecure` is true,
/// certificate validation is disabled (test/diagnostic use only).
pub fn build_client_config(ca_file: Option<&Path>, insecure: bool) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if let Some(path) = ca_file {
        let pem = fs::read(path).with_context(|| format!("reading CA bundle {:?}", path))?;
        let mut cursor = std::io::Cursor::new(pem);
        let certs = rustls_pemfile::certs(&mut cursor)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("parsing CA PEM")?;
        for cert in certs {
            roots
                .add(cert)
                .context("adding user CA to root store")?;
        }
    }

    let config = if insecure {
        // Danger: accept any server cert.  Documented in --help.
        let mut cfg = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.dangerous()
            .set_certificate_verifier(Arc::new(danger::AcceptAnyCert));
        cfg
    } else {
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    };

    Ok(Arc::new(config))
}

// ---------- "insecure" verifier (test-only) -----------------------------
mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct AcceptAnyCert;

    impl ServerCertVerifier for AcceptAnyCert {
        fn verify_server_cert(
            &self,
            _end: &CertificateDer<'_>,
            _ints: &[CertificateDer<'_>],
            _sn: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _m: &[u8],
            _c: &CertificateDer<'_>,
            _d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _m: &[u8],
            _c: &CertificateDer<'_>,
            _d: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }
}

// rustls-pemfile is small enough that we re-export only what we use.
// Adding it as an explicit dep would clutter Cargo.toml; pull it in
// transitively via rustls is not guaranteed, so add the small dep.
pub use rustls_pemfile;
