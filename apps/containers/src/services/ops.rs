//! Dispatch lifecycle, create, inspect, stats, and image jobs to one engine.

use crate::engines::{docker, incus};
use crate::models::{
    Action, CreateSpec, Engine, HostId, ImageDetail, InstanceSnap, LogChunk, NetSpec, ResourceSample, SnapAction, TermEvent,
    TermKind, VolumeSpec, WorkloadDetail,
};
use crate::prefs::RemoteProfile;
use tokio::sync::{mpsc, watch};

pub async fn operate(remote: &RemoteProfile, host: HostId, engine: Engine, name: &str, action: Action) -> Result<String, String> {
    match engine {
        Engine::Docker => docker::operate(host, remote, name, action).await,
        Engine::Incus => incus::operate(host, remote, name, action).await,
    }
}

pub async fn create(remote: &RemoteProfile, spec: &CreateSpec) -> Result<String, String> {
    match spec.engine {
        Engine::Docker => docker::create(remote, spec).await,
        Engine::Incus => incus::create(remote, spec).await,
    }
}

pub async fn inspect(remote: &RemoteProfile, host: HostId, engine: Engine, name: &str) -> Result<WorkloadDetail, String> {
    match engine {
        Engine::Docker => docker::inspect(host, remote, name).await,
        Engine::Incus => incus::inspect(host, remote, name).await,
    }
}

pub async fn stats(remote: &RemoteProfile, host: HostId, engine: Engine, name: &str, seq: u64) -> Result<ResourceSample, String> {
    match engine {
        Engine::Docker => docker::stats(host, remote, name, seq).await,
        Engine::Incus => incus::stats(host, remote, name, seq).await,
    }
}

pub async fn pull_image(remote: &RemoteProfile, host: HostId, engine: Engine, reference: &str) -> Result<String, String> {
    match engine {
        Engine::Docker => docker::pull_image(host, remote, reference).await,
        Engine::Incus => incus::pull_image(host, remote, reference).await,
    }
}

pub async fn inspect_image(remote: &RemoteProfile, host: HostId, engine: Engine, id: &str) -> Result<ImageDetail, String> {
    match engine {
        Engine::Docker => docker::inspect_image(host, remote, id).await,
        Engine::Incus => incus::inspect_image(host, remote, id).await,
    }
}

pub async fn remove_image(remote: &RemoteProfile, host: HostId, engine: Engine, id: &str) -> Result<String, String> {
    match engine {
        Engine::Docker => docker::remove_image(host, remote, id).await,
        Engine::Incus => incus::remove_image(host, remote, id).await,
    }
}

pub async fn create_network(remote: &RemoteProfile, spec: &NetSpec) -> Result<String, String> {
    match spec.engine {
        Engine::Docker => docker::create_network(spec.host, remote, &spec.name, &spec.driver).await,
        Engine::Incus => incus::create_network(spec.host, remote, &spec.name, &spec.driver).await,
    }
}

pub async fn remove_network(remote: &RemoteProfile, host: HostId, engine: Engine, name: &str) -> Result<String, String> {
    match engine {
        Engine::Docker => docker::remove_network(host, remote, name).await,
        Engine::Incus => incus::remove_network(host, remote, name).await,
    }
}

pub async fn create_volume(remote: &RemoteProfile, spec: &VolumeSpec) -> Result<String, String> {
    match spec.engine {
        Engine::Docker => docker::create_volume(spec.host, remote, &spec.name).await,
        Engine::Incus => {
            let pool = spec.pool.as_deref().unwrap_or("default");
            incus::create_volume(spec.host, remote, &spec.name, pool).await
        }
    }
}

pub async fn remove_volume(
    remote: &RemoteProfile,
    host: HostId,
    engine: Engine,
    name: &str,
    pool: Option<&str>,
) -> Result<String, String> {
    match engine {
        Engine::Docker => docker::remove_volume(host, remote, name).await,
        Engine::Incus => incus::remove_volume(host, remote, name, pool.unwrap_or("default")).await,
    }
}

pub async fn list_snapshots(remote: &RemoteProfile, host: HostId, instance: &str) -> Result<Vec<InstanceSnap>, String> {
    incus::list_snapshots(host, remote, instance).await
}

pub async fn snap_op(
    remote: &RemoteProfile,
    host: HostId,
    instance: &str,
    snap: &str,
    action: SnapAction,
) -> Result<String, String> {
    match action {
        SnapAction::Create => incus::create_snapshot(host, remote, instance, snap).await,
        SnapAction::Delete => incus::delete_snapshot(host, remote, instance, snap).await,
        SnapAction::Restore => incus::restore_snapshot(host, remote, instance, snap).await,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn follow_logs(
    remote: &RemoteProfile,
    host: HostId,
    engine: Engine,
    name: &str,
    seq: u64,
    follow: bool,
    chunks: mpsc::UnboundedSender<LogChunk>,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    match engine {
        Engine::Docker => docker::follow_logs(host, remote, name, seq, follow, chunks, cancel).await,
        Engine::Incus => incus::follow_logs(host, remote, name, seq, follow, chunks, cancel).await,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn session(
    remote: &RemoteProfile,
    host: HostId,
    engine: Engine,
    name: &str,
    kind: TermKind,
    events: mpsc::UnboundedReceiver<TermEvent>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    match (engine, kind) {
        (Engine::Docker, _) => docker::exec(host, remote, name, events, out, cancel).await,
        (Engine::Incus, kind) => incus::session(host, remote, name, kind, events, out, cancel).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_vm_create_is_rejected() {
        let spec = CreateSpec {
            host: HostId::Local,
            engine: Engine::Docker,
            kind: crate::models::Kind::VirtualMachine,
            name: "vm".into(),
            image: "ubuntu".into(),
            start: false,
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt.block_on(create(&RemoteProfile::default(), &spec)).unwrap_err();
        assert!(err.contains("virtual machines"));
    }
}
