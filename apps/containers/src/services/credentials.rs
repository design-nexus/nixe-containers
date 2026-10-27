//! Desktop Secret Service via `secret-tool`. The secret never implements a
//! readable [`Debug`], and these helpers do not write it to a file.

use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};

const APPLICATION: &str = "nyx-containers";
const ACCOUNT: &str = "remote";

/// A token or passphrase. Debug output is redacted.
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        anyhow::ensure!(
            !value.is_empty() && !value.chars().any(char::is_control),
            "the secret is empty or has control characters"
        );
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

pub fn secret_command(verb: &str, kind: &str) -> Command {
    let mut command = Command::new("secret-tool");
    command.arg(verb);
    if verb == "store" {
        command.arg(format!("--label={APPLICATION} {kind}"));
    }
    command.args(["application", APPLICATION, "account", ACCOUNT, "kind", kind]);
    command
}

pub fn store_secret(kind: &str, secret: &Secret) -> anyhow::Result<()> {
    let mut child = secret_command("store", kind).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(secret.expose().as_bytes())?;
    let out = child.wait_with_output()?;
    anyhow::ensure!(out.status.success(), "the keyring did not store the secret");
    Ok(())
}

#[allow(dead_code)]
pub fn lookup_secret(kind: &str) -> anyhow::Result<Secret> {
    let out = secret_command("lookup", kind).output()?;
    anyhow::ensure!(out.status.success(), "no secret is saved for this connection");
    let text = String::from_utf8(out.stdout)?;
    Secret::new(text.trim())
}

#[allow(dead_code)]
pub fn lookup_secret_optional(kind: &str) -> Option<Secret> {
    lookup_secret(kind).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_secret() {
        let secret = Secret::new("ssh-passphrase-value").unwrap();
        let shown = format!("{secret:?}");
        assert_eq!(shown, "Secret([redacted])");
        assert!(!shown.contains("ssh-passphrase"));
    }

    #[test]
    fn the_command_names_the_account_and_not_the_secret() {
        let command = secret_command("lookup", "ssh-passphrase");
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(command.get_program(), "secret-tool");
        assert_eq!(args, ["lookup", "application", "nyx-containers", "account", "remote", "kind", "ssh-passphrase"]);
    }
}
