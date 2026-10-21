//! Connection profile helpers used by the Connections page.

use crate::models::{ConnState, Engine, EngineStatus, HostId};
use crate::prefs::RemoteProfile;
use crate::security::ssh;

pub fn remote_ready(profile: &RemoteProfile) -> bool {
    profile.configured()
}

pub async fn test_remote(profile: &RemoteProfile) -> Vec<EngineStatus> {
    let mut out = Vec::new();
    if profile.docker_enabled {
        let (st, _) = crate::engines::docker::probe_remote(profile).await;
        out.push(st);
    } else {
        out.push(EngineStatus { host: HostId::Remote, engine: Engine::Docker, state: ConnState::Unconfigured });
    }
    if profile.incus_enabled {
        let (st, _) = crate::engines::incus::probe_remote(profile).await;
        out.push(st);
    } else {
        out.push(EngineStatus { host: HostId::Remote, engine: Engine::Incus, state: ConnState::Unconfigured });
    }
    out
}

#[allow(dead_code)]
pub fn probe_ssh_fingerprint(profile: &RemoteProfile) -> anyhow::Result<String> {
    let port = if profile.docker_ssh_port == 0 { 22 } else { profile.docker_ssh_port };
    Ok(ssh::probe(&profile.host, port)?.fingerprint)
}

pub fn remote_label(profile: &RemoteProfile) -> String {
    if profile.configured() { profile.display_name() } else { "Remote".into() }
}
