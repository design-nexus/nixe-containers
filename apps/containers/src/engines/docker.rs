//! Docker Engine API via Bollard.

use crate::models::{
    Action, ConnState, CreateSpec, Engine, EngineStatus, HostId, Image, ImageDetail, Kind, LogChunk, LogStream, NetRow,
    ResourceSample, RunState, TermEvent, TrustKind, VolumeRow, Workload, WorkloadDetail,
};
use crate::prefs::RemoteProfile;
use crate::security::ssh;
use bollard::Docker;
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptions, ListImagesOptionsBuilder,
    LogsOptionsBuilder, RemoveContainerOptionsBuilder, RemoveImageOptionsBuilder, StatsOptionsBuilder,
};
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};

pub fn socket_candidates_from_env(
    docker_host: Option<&str>,
    runtime_dir: Option<&str>,
    extra: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, EngineStatus> {
    if let Some(host) = docker_host {
        let host = host.trim();
        if host.starts_with("tcp://") || host.starts_with("http://") {
            return Err(EngineStatus {
                host: HostId::Local,
                engine: Engine::Docker,
                state: ConnState::Insecure {
                    reason: "DOCKER_HOST points at an unauthenticated TCP API. Set a unix socket or TLS endpoint.".into(),
                },
            });
        }
        if let Some(path) = host.strip_prefix("unix://") {
            return Ok(vec![PathBuf::from(path)]);
        }
    }
    let mut out = Vec::new();
    if let Some(runtime) = runtime_dir.filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(runtime).join("docker.sock"));
    }
    out.extend(extra);
    out.push(PathBuf::from("/var/run/docker.sock"));
    out.push(PathBuf::from("/run/docker.sock"));
    Ok(unique_keep_order(out))
}

fn unique_keep_order(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

pub fn local_socket_candidates() -> Result<Vec<PathBuf>, EngineStatus> {
    socket_candidates_from_env(
        std::env::var("DOCKER_HOST").ok().as_deref(),
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        None,
    )
}

fn permission_status(path: &Path) -> EngineStatus {
    EngineStatus::permission(
        HostId::Local,
        Engine::Docker,
        format!("Docker's socket is at {} but this user cannot access it. Add the user to the docker group.", path.display()),
    )
}

pub async fn probe_local() -> (EngineStatus, Vec<Workload>) {
    let candidates = match local_socket_candidates() {
        Ok(c) => c,
        Err(status) => return (status, Vec::new()),
    };
    let mut last_missing: Option<PathBuf> = None;
    let mut last_denied: Option<PathBuf> = None;
    for path in candidates {
        match path.metadata() {
            Err(_) => {
                last_missing = Some(path);
                continue;
            }
            Ok(_) => {
                if std::os::unix::net::UnixStream::connect(&path).is_err() {
                    last_denied = Some(path);
                    continue;
                }
            }
        }
        match connect_unix(&path) {
            Ok(docker) => match list(&docker, HostId::Local).await {
                Ok((detail, workloads)) => {
                    return (EngineStatus::connected(HostId::Local, Engine::Docker, detail), workloads);
                }
                Err(err) => {
                    return (EngineStatus::unavailable(HostId::Local, Engine::Docker, err), Vec::new());
                }
            },
            Err(err) if err.contains("Permission denied") => last_denied = Some(path),
            Err(err) => return (EngineStatus::unavailable(HostId::Local, Engine::Docker, err), Vec::new()),
        }
    }
    if let Some(path) = last_denied {
        return (permission_status(&path), Vec::new());
    }
    (
        EngineStatus::unavailable(
            HostId::Local,
            Engine::Docker,
            last_missing
                .map(|p| format!("No Docker socket at {}.", p.display()))
                .unwrap_or_else(|| "Docker is not available on this computer.".into()),
        ),
        Vec::new(),
    )
}

fn connect_unix(path: &Path) -> Result<Docker, String> {
    let addr = format!("unix://{}", path.display());
    Docker::connect_with_unix(&addr, 20, bollard::API_DEFAULT_VERSION).map_err(|e| e.to_string())
}

async fn list(docker: &Docker, host: HostId) -> Result<(String, Vec<Workload>), String> {
    docker.ping().await.map_err(|e| classify_docker_error(&e.to_string()))?;
    let version = docker.version().await.ok();
    let opts = ListContainersOptions { all: true, ..Default::default() };
    let containers = docker.list_containers(Some(opts)).await.map_err(|e| e.to_string())?;
    let workloads = containers
        .into_iter()
        .map(|c| {
            let name = c
                .names
                .as_ref()
                .and_then(|n| n.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .or_else(|| c.id.clone().map(|id| id.chars().take(12).collect()))
                .unwrap_or_else(|| "unnamed".into());
            let status = c.status.clone().unwrap_or_else(|| c.state.map(|s| s.to_string()).unwrap_or_else(|| "unknown".into()));
            let id = c.id.clone().unwrap_or_else(|| name.clone());
            let project = c.labels.as_ref().and_then(|l| l.get("com.docker.compose.project").cloned()).filter(|s| !s.is_empty());
            Workload {
                host,
                engine: Engine::Docker,
                kind: Kind::Container,
                name,
                id,
                state: RunState::from_status(&status),
                status,
                image: c.image,
                project,
            }
        })
        .collect();
    let ver = version.as_ref().and_then(|v| v.version.clone()).unwrap_or_else(|| "Docker".into());
    Ok((ver, workloads))
}

fn classify_docker_error(err: &str) -> String {
    if err.to_ascii_lowercase().contains("permission denied") {
        "This user cannot talk to the Docker socket. Add the user to the docker group.".into()
    } else {
        err.to_string()
    }
}

pub async fn probe_remote(profile: &RemoteProfile) -> (EngineStatus, Vec<Workload>) {
    if !profile.docker_enabled {
        return (EngineStatus { host: HostId::Remote, engine: Engine::Docker, state: ConnState::Unconfigured }, Vec::new());
    }
    if profile.host.trim().is_empty() {
        return (EngineStatus { host: HostId::Remote, engine: Engine::Docker, state: ConnState::Unconfigured }, Vec::new());
    }
    let host = profile.host.trim();
    let port = if profile.docker_ssh_port == 0 { 22 } else { profile.docker_ssh_port };
    let key = match ssh::probe(host, port) {
        Ok(k) => k,
        Err(err) => {
            return (EngineStatus::unavailable(HostId::Remote, Engine::Docker, format!("SSH host key: {err}")), Vec::new());
        }
    };
    if profile.ssh_host_fingerprint.trim().is_empty() {
        return (
            EngineStatus {
                host: HostId::Remote,
                engine: Engine::Docker,
                state: ConnState::Untrusted { fingerprint: key.fingerprint, kind: TrustKind::Ssh },
            },
            Vec::new(),
        );
    }
    if !ssh::fingerprints_match(&profile.ssh_host_fingerprint, &key.fingerprint) {
        return (
            EngineStatus::unavailable(
                HostId::Remote,
                Engine::Docker,
                format!(
                    "SSH host key changed. Stored {} but the server presented {}.",
                    profile.ssh_host_fingerprint, key.fingerprint
                ),
            ),
            Vec::new(),
        );
    }
    match connect_ssh(profile) {
        Ok(docker) => match list(&docker, HostId::Remote).await {
            Ok((detail, workloads)) => (EngineStatus::connected(HostId::Remote, Engine::Docker, detail), workloads),
            Err(err) => (EngineStatus::unavailable(HostId::Remote, Engine::Docker, err), Vec::new()),
        },
        Err(err) => (EngineStatus::unavailable(HostId::Remote, Engine::Docker, err), Vec::new()),
    }
}

async fn connect(host: HostId, remote: &RemoteProfile) -> Result<Docker, String> {
    match host {
        HostId::Local => local_docker().await,
        HostId::Remote => connect_ssh(remote),
    }
}

async fn local_docker() -> Result<Docker, String> {
    let candidates = local_socket_candidates().map_err(|st| match st.state {
        ConnState::Insecure { reason } | ConnState::Unavailable { reason } | ConnState::Permission { reason } => reason,
        _ => "Docker is not available on this computer.".into(),
    })?;
    let mut last_err = "Docker is not available on this computer.".to_string();
    for path in candidates {
        match connect_unix(&path) {
            Ok(docker) => {
                if docker.ping().await.is_ok() {
                    return Ok(docker);
                }
            }
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

fn connect_ssh(profile: &RemoteProfile) -> Result<Docker, String> {
    if !profile.docker_enabled {
        return Err("Remote Docker is not enabled.".into());
    }
    if profile.host.trim().is_empty() {
        return Err("No remote host is configured.".into());
    }
    let host = profile.host.trim();
    let port = if profile.docker_ssh_port == 0 { 22 } else { profile.docker_ssh_port };
    let key = ssh::probe(host, port).map_err(|e| format!("SSH host key: {e}"))?;
    if profile.ssh_host_fingerprint.trim().is_empty() {
        return Err(format!("Unknown SSH identity. Fingerprint {}", key.fingerprint));
    }
    if !ssh::fingerprints_match(&profile.ssh_host_fingerprint, &key.fingerprint) {
        return Err(format!(
            "SSH host key changed. Stored {} but the server presented {}.",
            profile.ssh_host_fingerprint, key.fingerprint
        ));
    }
    let user = if profile.docker_ssh_user.trim().is_empty() {
        std::env::var("USER").unwrap_or_else(|_| "root".into())
    } else {
        profile.docker_ssh_user.trim().to_string()
    };
    let addr = format!("ssh://{user}@{host}:{port}");
    let identity =
        if profile.docker_identity_file.trim().is_empty() { None } else { Some(profile.docker_identity_file.trim().to_string()) };
    Docker::connect_with_ssh(&addr, 20, bollard::API_DEFAULT_VERSION, identity).map_err(|e| e.to_string())
}

pub async fn operate(host: HostId, remote: &RemoteProfile, name: &str, action: Action) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    match action {
        Action::Start => docker.start_container(name, None::<bollard::query_parameters::StartContainerOptions>).await,
        Action::Stop => docker.stop_container(name, None::<bollard::query_parameters::StopContainerOptions>).await,
        Action::Restart => docker.restart_container(name, None::<bollard::query_parameters::RestartContainerOptions>).await,
        Action::Delete => {
            let opts = RemoveContainerOptionsBuilder::default().force(true).build();
            docker.remove_container(name, Some(opts)).await
        }
    }
    .map_err(|e| e.to_string())?;
    Ok(format!("{} {name}", action.done()))
}

pub async fn create(remote: &RemoteProfile, spec: &CreateSpec) -> Result<String, String> {
    if spec.kind == Kind::VirtualMachine {
        return Err("Docker does not run virtual machines.".into());
    }
    let docker = connect(spec.host, remote).await?;
    let opts = CreateContainerOptionsBuilder::default().name(&spec.name).build();
    let body = ContainerCreateBody { image: Some(spec.image.clone()), ..Default::default() };
    docker.create_container(Some(opts), body).await.map_err(|e| e.to_string())?;
    if spec.start {
        docker
            .start_container(&spec.name, None::<bollard::query_parameters::StartContainerOptions>)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(format!("Created and started {}", spec.name));
    }
    Ok(format!("Created {}", spec.name))
}

pub async fn inspect(host: HostId, remote: &RemoteProfile, name: &str) -> Result<WorkloadDetail, String> {
    let docker = connect(host, remote).await?;
    let info = docker.inspect_container(name, None).await.map_err(|e| e.to_string())?;
    let status = info.state.as_ref().and_then(|s| s.status.map(|st| st.to_string())).unwrap_or_else(|| "unknown".into());
    let command = match (&info.path, &info.args) {
        (Some(path), Some(args)) if !args.is_empty() => Some(format!("{path} {}", args.join(" "))),
        (Some(path), _) => Some(path.clone()),
        _ => None,
    };
    let ip = info
        .network_settings
        .as_ref()
        .and_then(|n| n.networks.as_ref())
        .and_then(|nets| nets.values().find_map(|e| e.ip_address.clone().filter(|s| !s.is_empty())));
    let mut extra = Vec::new();
    if let Some(id) = info.id.clone() {
        extra.push(("ID".into(), id.chars().take(12).collect()));
    }
    if let Some(n) = info.restart_count {
        extra.push(("Restarts".into(), n.to_string()));
    }
    if let Some(p) = info.platform.clone() {
        extra.push(("Platform".into(), p));
    }
    let mut detail = WorkloadDetail {
        host,
        engine: Engine::Docker,
        kind: Kind::Container,
        name: info.name.as_deref().map(|n| n.trim_start_matches('/').to_string()).unwrap_or_else(|| name.to_string()),
        status: status.clone(),
        state: RunState::from_status(&status),
        image: info.config.as_ref().and_then(|c| c.image.clone()).or(info.image.clone()),
        created: info.created.clone(),
        command,
        ip,
        cpu: None,
        memory_used: None,
        memory_limit: None,
        extra,
        snapshots: Vec::new(),
    };
    if detail.state == RunState::Running
        && let Ok(sample) = stats_on(&docker, host, &detail.name, 0).await
    {
        detail.cpu = sample.cpu;
        detail.memory_used = sample.memory_used;
        detail.memory_limit = sample.memory_limit;
    }
    Ok(detail)
}

pub async fn stats(host: HostId, remote: &RemoteProfile, name: &str, seq: u64) -> Result<ResourceSample, String> {
    let docker = connect(host, remote).await?;
    stats_on(&docker, host, name, seq).await
}

async fn stats_on(docker: &Docker, host: HostId, name: &str, seq: u64) -> Result<ResourceSample, String> {
    let opts = StatsOptionsBuilder::default().stream(false).build();
    let mut stream = docker.stats(name, Some(opts));
    let stats = stream.next().await.ok_or_else(|| "Docker returned no stats.".to_string())?.map_err(|e| e.to_string())?;
    Ok(ResourceSample {
        host,
        engine: Engine::Docker,
        name: name.into(),
        cpu: cpu_percent(&stats),
        memory_used: stats.memory_stats.as_ref().and_then(|m| m.usage),
        memory_limit: stats.memory_stats.as_ref().and_then(|m| m.limit),
        seq,
    })
}

fn cpu_percent(stats: &bollard::models::ContainerStatsResponse) -> Option<f64> {
    let cpu = stats.cpu_stats.as_ref()?;
    let pre = stats.precpu_stats.as_ref()?;
    let total = cpu.cpu_usage.as_ref()?.total_usage?;
    let pret = pre.cpu_usage.as_ref()?.total_usage?;
    let sys = cpu.system_cpu_usage?;
    let presys = pre.system_cpu_usage?;
    let delta = total.saturating_sub(pret) as f64;
    let sysdelta = sys.saturating_sub(presys) as f64;
    if sysdelta <= 0.0 {
        return None;
    }
    let ncpus = cpu.online_cpus.unwrap_or(1).max(1) as f64;
    Some((delta / sysdelta) * ncpus * 100.0)
}

pub async fn list_images(host: HostId, remote: &RemoteProfile) -> Result<Vec<Image>, String> {
    let docker = connect(host, remote).await?;
    let opts = ListImagesOptionsBuilder::default().all(false).build();
    let images = docker.list_images(Some(opts)).await.map_err(|e| e.to_string())?;
    Ok(images
        .into_iter()
        .map(|img| {
            let name = img
                .repo_tags
                .iter()
                .find(|t| *t != "<none>:<none>")
                .cloned()
                .unwrap_or_else(|| img.id.chars().skip(7).take(12).collect());
            Image {
                host,
                engine: Engine::Docker,
                id: img.id.clone(),
                name,
                size: img.size.max(0) as u64,
                created: Some(img.created.to_string()),
            }
        })
        .collect())
}

pub async fn pull_image(host: HostId, remote: &RemoteProfile, reference: &str) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    let opts = CreateImageOptionsBuilder::default().from_image(reference).build();
    let mut stream = docker.create_image(Some(opts), None, None);
    let mut last_err: Option<String> = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(_) => last_err = None,
            Err(err) => last_err = Some(err.to_string()),
        }
    }
    if let Some(err) = last_err {
        return Err(err);
    }
    Ok(format!("Pulled {reference}"))
}

pub async fn inspect_image(host: HostId, remote: &RemoteProfile, id: &str) -> Result<ImageDetail, String> {
    let docker = connect(host, remote).await?;
    let info = docker.inspect_image(id).await.map_err(|e| e.to_string())?;
    let name = info.repo_tags.as_ref().and_then(|t| t.first()).cloned().unwrap_or_else(|| id.chars().take(16).collect());
    let mut extra = Vec::new();
    if let Some(arch) = info.architecture.clone() {
        extra.push(("Architecture".into(), arch));
    }
    if let Some(os) = info.os.clone() {
        extra.push(("OS".into(), os));
    }
    if let Some(author) = info.author.clone().filter(|s| !s.is_empty()) {
        extra.push(("Author".into(), author));
    }
    if let Some(id) = info.id.clone() {
        extra.push(("ID".into(), id));
    }
    Ok(ImageDetail {
        image: Image {
            host,
            engine: Engine::Docker,
            id: info.id.clone().unwrap_or_else(|| id.to_string()),
            name,
            size: info.size.unwrap_or(0).max(0) as u64,
            created: info.created.clone(),
        },
        extra,
    })
}

pub async fn host_disk(host: HostId, remote: &RemoteProfile) -> Result<(u64, Option<u64>), String> {
    let docker = connect(host, remote).await?;
    let df = docker.df(None::<bollard::query_parameters::DataUsageOptions>).await.map_err(|e| e.to_string())?;
    let mut used = 0u64;
    if let Some(u) = df.image_usage.and_then(|u| u.total_size) {
        used += u.max(0) as u64;
    }
    if let Some(u) = df.container_usage.and_then(|u| u.total_size) {
        used += u.max(0) as u64;
    }
    if let Some(u) = df.volume_usage.and_then(|u| u.total_size) {
        used += u.max(0) as u64;
    }
    if let Some(u) = df.build_cache_usage.and_then(|u| u.total_size) {
        used += u.max(0) as u64;
    }
    Ok((used, None))
}

pub async fn host_memory(host: HostId, remote: &RemoteProfile) -> Result<Option<u64>, String> {
    let docker = connect(host, remote).await?;
    let info = docker.info().await.map_err(|e| e.to_string())?;
    Ok(info.mem_total.filter(|n| *n > 0).map(|n| n as u64))
}

pub fn builtin_network(name: &str) -> bool {
    matches!(name, "bridge" | "host" | "none" | "docker_gwbridge")
}

pub async fn list_networks(host: HostId, remote: &RemoteProfile) -> Result<Vec<NetRow>, String> {
    let docker = connect(host, remote).await?;
    let nets = docker.list_networks(None).await.map_err(|e| e.to_string())?;
    Ok(nets
        .into_iter()
        .filter_map(|n| {
            let name = n.name?;
            let subnet = n.ipam.and_then(|i| i.config).and_then(|c| c.into_iter().find_map(|x| x.subnet));
            Some(NetRow {
                host,
                engine: Engine::Docker,
                name: name.clone(),
                driver: n.driver.unwrap_or_default(),
                subnet,
                used_by: 0,
                removable: !builtin_network(&name),
            })
        })
        .collect())
}

pub async fn create_network(host: HostId, remote: &RemoteProfile, name: &str, driver: &str) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    let driver = if driver.trim().is_empty() { "bridge" } else { driver.trim() };
    docker
        .create_network(bollard::models::NetworkCreateRequest {
            name: name.to_string(),
            driver: Some(driver.to_string()),
            ..Default::default()
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Created {name}"))
}

pub async fn remove_network(host: HostId, remote: &RemoteProfile, name: &str) -> Result<String, String> {
    if builtin_network(name) {
        return Err(format!("Cannot remove Docker's {name} network."));
    }
    let docker = connect(host, remote).await?;
    docker.remove_network(name).await.map_err(|e| e.to_string())?;
    Ok(format!("Removed {name}"))
}

pub async fn list_volumes(host: HostId, remote: &RemoteProfile) -> Result<Vec<VolumeRow>, String> {
    let docker = connect(host, remote).await?;
    let resp = docker
        .list_volumes(None::<bollard::query_parameters::ListVolumesOptions>)
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|v| {
            let size_label = v
                .usage_data
                .as_ref()
                .and_then(|u| if u.size >= 0 { Some(crate::models::format_bytes(u.size as u64)) } else { None })
                .unwrap_or_else(|| "—".into());
            let used_by = v.usage_data.as_ref().map(|u| u.ref_count.max(0) as usize).unwrap_or(0);
            VolumeRow {
                host,
                engine: Engine::Docker,
                name: v.name,
                kind: v.driver,
                pool: None,
                size_label,
                used_by,
                removable: true,
            }
        })
        .collect())
}

pub async fn create_volume(host: HostId, remote: &RemoteProfile, name: &str) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    docker
        .create_volume(bollard::models::VolumeCreateRequest { name: Some(name.to_string()), ..Default::default() })
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("Created {name}"))
}

pub async fn remove_volume(host: HostId, remote: &RemoteProfile, name: &str) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    let opts = bollard::query_parameters::RemoveVolumeOptionsBuilder::default().force(false).build();
    docker.remove_volume(name, Some(opts)).await.map_err(|e| e.to_string())?;
    Ok(format!("Removed {name}"))
}

pub async fn remove_image(host: HostId, remote: &RemoteProfile, id: &str) -> Result<String, String> {
    let docker = connect(host, remote).await?;
    let opts = RemoveImageOptionsBuilder::default().force(false).build();
    docker.remove_image(id, Some(opts), None).await.map_err(|e| e.to_string())?;
    Ok(format!("Removed {id}"))
}

pub async fn follow_logs(
    host: HostId,
    remote: &RemoteProfile,
    name: &str,
    seq: u64,
    follow: bool,
    chunks: mpsc::UnboundedSender<LogChunk>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let docker = connect(host, remote).await?;
    let opts = LogsOptionsBuilder::default().follow(follow).stdout(true).stderr(true).timestamps(true).tail("2000").build();
    let mut stream = docker.logs(name, Some(opts));
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
            item = stream.next() => {
                let Some(item) = item else { break };
                let out = item.map_err(|e| e.to_string())?;
                let (stream_kind, text) = match &out {
                    bollard::container::LogOutput::StdErr { .. } => (LogStream::Stderr, out.to_string()),
                    _ => (LogStream::Stdout, out.to_string()),
                };
                if chunks.send(LogChunk { seq, stream: stream_kind, text }).is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

pub async fn exec(
    host: HostId,
    remote: &RemoteProfile,
    name: &str,
    mut events: mpsc::UnboundedReceiver<TermEvent>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let docker = connect(host, remote).await?;
    let config = CreateExecOptions::<String> {
        attach_stdin: Some(true),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        tty: Some(true),
        cmd: Some(vec!["/bin/sh".into(), "-c".into(), "exec bash || exec sh".into()]),
        env: Some(vec!["TERM=xterm-256color".into()]),
        detach_keys: None,
        privileged: None,
        user: None,
        working_dir: None,
    };
    let created = docker.create_exec(name, config).await.map_err(|e| e.to_string())?;
    let started = docker
        .start_exec(&created.id, Some(StartExecOptions { tty: true, ..Default::default() }))
        .await
        .map_err(|e| e.to_string())?;
    let StartExecResults::Attached { mut output, mut input } = started else {
        return Err("Docker exec detached instead of attaching.".into());
    };
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
            event = events.recv() => {
                match event {
                    Some(TermEvent::Data(bytes)) => {
                        if input.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(TermEvent::Resize { cols, rows }) => {
                        let _ = docker
                            .resize_exec(&created.id, ResizeExecOptions { height: rows, width: cols })
                            .await;
                    }
                    Some(TermEvent::Close) | None => break,
                }
            }
            item = output.next() => {
                let Some(item) = item else { break };
                let chunk = item.map_err(|e| e.to_string())?;
                if out.send(chunk.into_bytes().to_vec()).is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_docker_host_is_the_only_candidate() {
        let paths = socket_candidates_from_env(Some("unix:///tmp/docker.sock"), Some("/run/user/1000"), None).unwrap();
        assert_eq!(paths, vec![PathBuf::from("/tmp/docker.sock")]);
    }

    #[test]
    fn tcp_docker_host_is_refused() {
        let err = socket_candidates_from_env(Some("tcp://127.0.0.1:2375"), None, None).unwrap_err();
        assert!(matches!(err.state, ConnState::Insecure { .. }));
    }

    #[test]
    fn runtime_dir_comes_before_system_socket() {
        let paths = socket_candidates_from_env(None, Some("/run/user/1000"), None).unwrap();
        assert_eq!(paths[0], PathBuf::from("/run/user/1000/docker.sock"));
        assert!(paths.iter().any(|p| p == Path::new("/var/run/docker.sock")));
    }

    #[test]
    fn builtin_networks_cannot_be_removed() {
        assert!(builtin_network("bridge"));
        assert!(builtin_network("host"));
        assert!(builtin_network("none"));
        assert!(builtin_network("docker_gwbridge"));
        assert!(!builtin_network("appnet"));
    }
}
