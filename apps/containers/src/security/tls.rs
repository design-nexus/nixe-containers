//! TLS server-certificate fingerprint pinning for Incus HTTPS.

use rustls::DigitallySignedStruct;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub fn fingerprint_der(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    format!("sha256:{}", hex(&digest))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn fingerprints_match(stored: &str, found: &str) -> bool {
    normalize(stored) == normalize(found) && !stored.trim().is_empty()
}

fn normalize(fp: &str) -> String {
    fp.trim().to_ascii_lowercase().replace(':', "").replace("sha256", "")
}

#[derive(Debug)]
pub struct PinVerifier {
    expected: Option<String>,
    seen: std::sync::Mutex<Option<String>>,
}

impl PinVerifier {
    pub fn new(expected: Option<String>) -> Arc<Self> {
        Arc::new(Self { expected: expected.filter(|s| !s.trim().is_empty()), seen: std::sync::Mutex::new(None) })
    }

    pub fn seen(&self) -> Option<String> {
        self.seen.lock().ok().and_then(|g| g.clone())
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let found = fingerprint_der(end_entity.as_ref());
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(found.clone());
        }
        match &self.expected {
            Some(stored) if fingerprints_match(stored, &found) => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::General("certificate fingerprint does not match the stored pin".into())),
            None => Err(rustls::Error::General(format!("untrusted-cert:{found}"))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let provider = CryptoProvider::get_default().ok_or_else(|| rustls::Error::General("no crypto provider".into()))?;
        verify_tls12_signature(message, cert, dss, &provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let provider = CryptoProvider::get_default().ok_or_else(|| rustls::Error::General("no crypto provider".into()))?;
        verify_tls13_signature(message, cert, dss, &provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        CryptoProvider::get_default().map(|p| p.signature_verification_algorithms.supported_schemes()).unwrap_or_default()
    }
}

pub fn untrusted_fingerprint(err: &str) -> Option<String> {
    err.split_once("untrusted-cert:").map(|(_, rest)| rest.trim().trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_hex() {
        let fp = fingerprint_der(&[0, 1, 2, 3]);
        assert!(fp.starts_with("sha256:"));
        assert_eq!(fp.len(), "sha256:".len() + 64);
        assert!(fingerprints_match(&fp, &fp));
    }

    #[test]
    fn untrusted_error_yields_fingerprint() {
        let fp = untrusted_fingerprint("untrusted-cert:sha256:abcd").unwrap();
        assert_eq!(fp, "sha256:abcd");
    }
}
