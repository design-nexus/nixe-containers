//! SSH host-key fingerprints. Unknown keys are refused until the user accepts them.

use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    pub fingerprint: String,
    pub algorithm: String,
}

/// Fetch the remote host key and return its OpenSSH SHA256 fingerprint.
pub fn probe(host: &str, port: u16) -> anyhow::Result<HostKey> {
    anyhow::ensure!(!host.trim().is_empty(), "hostname is empty");
    let target = if port == 22 { host.to_string() } else { format!("[{host}]:{port}") };
    let out = Command::new("ssh-keyscan").args(["-T", "5", "-t", "ed25519,ecdsa,rsa", "-p", &port.to_string(), host]).output()?;
    anyhow::ensure!(out.status.success() || !out.stdout.is_empty(), "could not read the SSH host key for {host}");
    let text = String::from_utf8_lossy(&out.stdout);
    parse_keyscan(&text, &target).ok_or_else(|| anyhow::anyhow!("no SSH host key was advertised for {host}"))
}

pub fn parse_keyscan(text: &str, _target: &str) -> Option<HostKey> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let _name = parts.next()?;
        let algorithm = parts.next()?.to_string();
        let blob = parts.next()?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(blob).ok()?;
        let digest = Sha256::digest(&bytes);
        let fingerprint = format!("SHA256:{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest));
        return Some(HostKey { fingerprint, algorithm });
    }
    None
}

pub fn fingerprints_match(stored: &str, found: &str) -> bool {
    normalize(stored) == normalize(found) && !stored.trim().is_empty()
}

fn normalize(fp: &str) -> String {
    fp.trim().to_ascii_uppercase().replace('=', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ed25519_keyscan_line() {
        // ssh-ed25519 with a known blob; fingerprint is SHA256 of the decoded key.
        let blob = base64::engine::general_purpose::STANDARD.encode([0u8; 32]);
        let line = format!("example.com ssh-ed25519 {blob}");
        let key = parse_keyscan(&line, "example.com").unwrap();
        assert_eq!(key.algorithm, "ssh-ed25519");
        assert!(key.fingerprint.starts_with("SHA256:"));
        assert!(fingerprints_match(&key.fingerprint, &key.fingerprint));
    }

    #[test]
    fn empty_stored_fingerprint_does_not_match() {
        assert!(!fingerprints_match("", "SHA256:abc"));
    }
}
