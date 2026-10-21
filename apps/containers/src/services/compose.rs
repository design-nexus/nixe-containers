//! Docker Compose plugin CLI. Allowed exception: Compose has no stable engine API
//! for project up/down of services that are not already created.

use crate::models::{Action, HostId};
use tokio::process::Command;

pub async fn run(host: HostId, project: &str, dir: &str, file: &str, action: Action) -> Result<String, String> {
    if host != HostId::Local {
        return Err("Compose YAML deploy runs on this computer.".into());
    }
    if project.trim().is_empty() {
        return Err("A project name is required.".into());
    }
    if dir.trim().is_empty() || file.trim().is_empty() {
        return Err("A project directory and compose file are required.".into());
    }
    let verb: &[&str] = match action {
        Action::Start => &["up", "-d"],
        Action::Stop => &["down"],
        Action::Restart => &["restart"],
        Action::Delete => return Err("Compose delete is not supported.".into()),
    };
    let output = Command::new("docker")
        .args(["compose", "--project-name", project, "--project-directory", dir, "-f", file])
        .args(verb)
        .output()
        .await
        .map_err(|e| format!("docker compose: {e}"))?;
    if output.status.success() {
        Ok(format!("{} stack {project}", action.done()))
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        let out = String::from_utf8_lossy(&output.stdout);
        let msg = if err.trim().is_empty() { out.trim() } else { err.trim() };
        Err(if msg.is_empty() { format!("docker compose failed for {project}") } else { msg.to_string() })
    }
}

pub fn validate_yaml(text: &str) -> Result<(), String> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| format!("Invalid YAML: {e}"))?;
    if value.is_null() {
        return Err("Compose file is empty.".into());
    }
    Ok(())
}

pub const TEMPLATE: &str = "\
services:
  app:
    image: nginx:latest
    ports:
      - \"8080:80\"
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_valid_yaml() {
        validate_yaml(TEMPLATE).unwrap();
    }

    #[test]
    fn empty_yaml_is_rejected() {
        assert!(validate_yaml("").is_err());
        assert!(validate_yaml("[").is_err());
    }
}
