//! Merge engine probes into one snapshot.

use crate::engines::{docker, incus};
use crate::models::{Engine, EngineStatus, HostId, Snapshot};
use crate::prefs::RemoteProfile;

pub async fn collect(remote: &RemoteProfile) -> Snapshot {
    let mut snapshot = Snapshot::default();

    let (docker_local, docker_workloads) = docker::probe_local().await;
    extras(&mut snapshot, &docker_local, remote).await;
    push(&mut snapshot, docker_local, docker_workloads);

    let (incus_local, incus_workloads) = incus::probe_local().await;
    extras(&mut snapshot, &incus_local, remote).await;
    push(&mut snapshot, incus_local, incus_workloads);

    if remote.configured() {
        if remote.docker_enabled {
            let (st, list) = docker::probe_remote(remote).await;
            extras(&mut snapshot, &st, remote).await;
            push(&mut snapshot, st, list);
        } else {
            snapshot.engines.push(EngineStatus {
                host: HostId::Remote,
                engine: Engine::Docker,
                state: crate::models::ConnState::Unconfigured,
            });
        }
        if remote.incus_enabled {
            let (st, list) = incus::probe_remote(remote).await;
            extras(&mut snapshot, &st, remote).await;
            push(&mut snapshot, st, list);
        } else {
            snapshot.engines.push(EngineStatus {
                host: HostId::Remote,
                engine: Engine::Incus,
                state: crate::models::ConnState::Unconfigured,
            });
        }
    }

    snapshot.workloads.sort_by_key(|a| a.name.to_ascii_lowercase());
    snapshot.images.sort_by_key(|a| a.name.to_ascii_lowercase());
    snapshot.networks.sort_by_key(|a| a.name.to_ascii_lowercase());
    snapshot.volumes.sort_by_key(|a| a.name.to_ascii_lowercase());
    snapshot
}

async fn extras(snapshot: &mut Snapshot, status: &EngineStatus, remote: &RemoteProfile) {
    if !status.is_connected() {
        return;
    }
    match status.engine {
        Engine::Docker => {
            if let Ok(list) = docker::list_images(status.host, remote).await {
                snapshot.images.extend(list);
            }
            if let Ok(list) = docker::list_networks(status.host, remote).await {
                snapshot.networks.extend(list);
            }
            if let Ok(list) = docker::list_volumes(status.host, remote).await {
                snapshot.volumes.extend(list);
            }
        }
        Engine::Incus => {
            if let Ok(list) = incus::list_images(status.host, remote).await {
                snapshot.images.extend(list);
            }
            if let Ok(list) = incus::list_networks(status.host, remote).await {
                snapshot.networks.extend(list);
            }
            if let Ok(list) = incus::list_volumes(status.host, remote).await {
                snapshot.volumes.extend(list);
            }
        }
    }
}

fn push(snapshot: &mut Snapshot, status: EngineStatus, workloads: Vec<crate::models::Workload>) {
    if let crate::models::ConnState::Unavailable { reason }
    | crate::models::ConnState::Permission { reason }
    | crate::models::ConnState::Insecure { reason } = &status.state
    {
        snapshot.errors.push(format!("{} {}: {reason}", status.host.label(), status.engine.label()));
    }
    snapshot.engines.push(status);
    snapshot.workloads.extend(workloads);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Kind, RunState, Workload};

    #[test]
    fn merge_sorts_by_name() {
        let mut snapshot = Snapshot::default();
        snapshot.workloads.push(Workload {
            host: HostId::Local,
            engine: Engine::Docker,
            kind: Kind::Container,
            name: "zeta".into(),
            id: "zeta".into(),
            status: "running".into(),
            state: RunState::Running,
            image: None,
            project: None,
            project_dir: None,
            compose_file: None,
        });
        snapshot.workloads.push(Workload {
            host: HostId::Local,
            engine: Engine::Docker,
            kind: Kind::Container,
            name: "alpha".into(),
            id: "alpha".into(),
            status: "exited".into(),
            state: RunState::Stopped,
            image: None,
            project: None,
            project_dir: None,
            compose_file: None,
        });
        snapshot.workloads.sort_by_key(|a| a.name.to_ascii_lowercase());
        assert_eq!(snapshot.workloads[0].name, "alpha");
        assert_eq!(snapshot.running(crate::models::HostFilter::All, None), 1);
        assert_eq!(snapshot.stopped(crate::models::HostFilter::All, None), 1);
    }
}
