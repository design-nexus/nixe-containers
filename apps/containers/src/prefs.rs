//! This app's own preferences (`~/.config/nyx-containers/settings.toml`).

pub use nyx_theme::ThemeMode;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteProfile {
    pub name: String,
    pub host: String,
    pub docker_enabled: bool,
    pub docker_ssh_user: String,
    pub docker_ssh_port: u16,
    pub docker_identity_file: String,
    pub docker_socket: String,
    pub ssh_host_fingerprint: String,
    pub incus_enabled: bool,
    pub incus_url: String,
    pub incus_client_cert: String,
    pub incus_client_key: String,
    pub incus_server_fingerprint: String,
}

impl Default for RemoteProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            docker_enabled: true,
            docker_ssh_user: String::new(),
            docker_ssh_port: 22,
            docker_identity_file: String::new(),
            docker_socket: "/var/run/docker.sock".into(),
            ssh_host_fingerprint: String::new(),
            incus_enabled: true,
            incus_url: String::new(),
            incus_client_cert: String::new(),
            incus_client_key: String::new(),
            incus_server_fingerprint: String::new(),
        }
    }
}

impl RemoteProfile {
    pub fn configured(&self) -> bool {
        !self.host.trim().is_empty() && (self.docker_enabled || self.incus_enabled)
    }

    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() { self.host.trim().to_string() } else { name.to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub reduce_motion: bool,
    pub glow: bool,
    pub sidebar_collapsed: bool,
    pub last_section: String,
    pub remote: RemoteProfile,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            reduce_motion: false,
            glow: true,
            sidebar_collapsed: false,
            last_section: "overview".into(),
            remote: RemoteProfile::default(),
        }
    }
}

nyx_core::prefs_store!(Prefs);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_needs_a_host_to_be_configured() {
        let mut remote = RemoteProfile::default();
        assert!(!remote.configured());
        remote.host = "hades.local".into();
        assert!(remote.configured());
    }

    #[test]
    fn prefs_round_trip_toml() {
        let prefs = Prefs { last_section: "connections".into(), ..Prefs::default() };
        let text = toml::to_string(&prefs).unwrap();
        let back: Prefs = toml::from_str(&text).unwrap();
        assert_eq!(back.last_section, "connections");
        assert_eq!(back.remote.docker_ssh_port, 22);
    }
}
