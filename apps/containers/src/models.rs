//! Shared types for hosts, engines, and workloads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostId {
    Local,
    Remote,
}

impl HostId {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostFilter {
    All,
    Local,
    Remote,
}

impl HostFilter {
    pub fn matches(self, host: HostId) -> bool {
        match self {
            Self::All => true,
            Self::Local => host == HostId::Local,
            Self::Remote => host == HostId::Remote,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Docker,
    Incus,
}

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Incus => "Incus",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Container,
    VirtualMachine,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Container => "Container",
            Self::VirtualMachine => "Virtual machine",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunState {
    Running,
    Stopped,
    Paused,
    Other,
}

impl RunState {
    pub fn from_status(status: &str) -> Self {
        let s = status.to_ascii_lowercase();
        if s.contains("running") || s == "up" {
            Self::Running
        } else if s.contains("paused") {
            Self::Paused
        } else if s.contains("exited") || s.contains("stopped") || s.contains("created") || s.contains("dead") {
            Self::Stopped
        } else {
            Self::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Stopped => "Stopped",
            Self::Paused => "Paused",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Delete,
}

impl Action {
    pub fn gerund(self) -> &'static str {
        match self {
            Self::Start => "Starting",
            Self::Stop => "Stopping",
            Self::Restart => "Restarting",
            Self::Delete => "Deleting",
        }
    }

    pub fn done(self) -> &'static str {
        match self {
            Self::Start => "Started",
            Self::Stop => "Stopped",
            Self::Restart => "Restarted",
            Self::Delete => "Deleted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Workload {
    pub host: HostId,
    pub engine: Engine,
    pub kind: Kind,
    pub name: String,
    pub id: String,
    pub status: String,
    pub state: RunState,
    pub image: Option<String>,
    pub project: Option<String>,
    pub project_dir: Option<String>,
    pub compose_file: Option<String>,
}

pub fn compose_file_from_labels(working_dir: Option<&str>, config_files: Option<&str>) -> Option<String> {
    let first = config_files?.split(',').map(str::trim).find(|s| !s.is_empty())?;
    let path = std::path::Path::new(first);
    if path.is_absolute() {
        Some(first.to_string())
    } else {
        let dir = working_dir?.trim_end_matches('/');
        if dir.is_empty() {
            None
        } else {
            Some(format!("{dir}/{first}"))
        }
    }
}

impl Workload {
    pub fn target_id(&self) -> &str {
        if self.id.is_empty() { &self.name } else { &self.id }
    }
}

#[derive(Debug, Clone)]
pub struct CreateSpec {
    pub host: HostId,
    pub engine: Engine,
    pub kind: Kind,
    pub name: String,
    pub image: String,
    pub start: bool,
}

#[derive(Debug, Clone)]
pub struct WorkloadDetail {
    pub host: HostId,
    pub engine: Engine,
    pub kind: Kind,
    pub name: String,
    pub status: String,
    pub state: RunState,
    pub image: Option<String>,
    pub created: Option<String>,
    pub command: Option<String>,
    pub ip: Option<String>,
    pub cpu: Option<f64>,
    pub memory_used: Option<u64>,
    pub memory_limit: Option<u64>,
    pub extra: Vec<(String, String)>,
    pub snapshots: Vec<InstanceSnap>,
}

#[derive(Debug, Clone)]
pub struct ResourceSample {
    pub host: HostId,
    pub engine: Engine,
    pub name: String,
    pub cpu: Option<f64>,
    pub memory_used: Option<u64>,
    pub memory_limit: Option<u64>,
    pub seq: u64,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub host: HostId,
    pub engine: Engine,
    pub id: String,
    pub name: String,
    pub size: u64,
    pub created: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImageDetail {
    pub image: Image,
    pub extra: Vec<(String, String)>,
}

pub fn format_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let n = n as f64;
    if n < K {
        format!("{n:.0} B")
    } else if n < K * K {
        format!("{:.1} KiB", n / K)
    } else if n < K * K * K {
        format!("{:.1} MiB", n / K / K)
    } else {
        format!("{:.1} GiB", n / K / K / K)
    }
}

pub fn format_cpu(cpu: Option<f64>) -> String {
    match cpu {
        Some(v) => format!("{v:.1}%"),
        None => "—".into(),
    }
}

pub fn format_memory(used: Option<u64>, limit: Option<u64>) -> String {
    match (used, limit) {
        (Some(u), Some(l)) if l > 0 => format!("{} / {}", format_bytes(u), format_bytes(l)),
        (Some(u), _) => format_bytes(u),
        _ => "—".into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
    Console,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFilter {
    All,
    Stdout,
    Stderr,
}

impl LogFilter {
    pub fn accepts(self, stream: LogStream) -> bool {
        match self {
            Self::All => true,
            Self::Stdout => matches!(stream, LogStream::Stdout | LogStream::Console),
            Self::Stderr => stream == LogStream::Stderr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogChunk {
    pub seq: u64,
    pub stream: LogStream,
    pub text: String,
}

pub fn log_visible(chunk: &LogChunk, filter: LogFilter, query: &str) -> bool {
    if !filter.accepts(chunk.stream) {
        return false;
    }
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    chunk.text.to_ascii_lowercase().contains(&q.to_ascii_lowercase())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermKind {
    Exec,
    Console,
}

#[derive(Debug, Clone)]
pub enum TermEvent {
    Data(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Close,
}

#[derive(Debug, Clone)]
pub enum ConnState {
    Connected { detail: String },
    Unavailable { reason: String },
    Permission { reason: String },
    Unconfigured,
    Untrusted { fingerprint: String, kind: TrustKind },
    Insecure { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustKind {
    Ssh,
    Tls,
}

#[derive(Debug, Clone)]
pub struct EngineStatus {
    pub host: HostId,
    pub engine: Engine,
    pub state: ConnState,
}

impl EngineStatus {
    pub fn connected(host: HostId, engine: Engine, detail: impl Into<String>) -> Self {
        Self { host, engine, state: ConnState::Connected { detail: detail.into() } }
    }

    pub fn unavailable(host: HostId, engine: Engine, reason: impl Into<String>) -> Self {
        Self { host, engine, state: ConnState::Unavailable { reason: reason.into() } }
    }

    pub fn permission(host: HostId, engine: Engine, reason: impl Into<String>) -> Self {
        Self { host, engine, state: ConnState::Permission { reason: reason.into() } }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, ConnState::Connected { .. })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub engines: Vec<EngineStatus>,
    pub workloads: Vec<Workload>,
    pub images: Vec<Image>,
    pub networks: Vec<NetRow>,
    pub volumes: Vec<VolumeRow>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NetRow {
    pub host: HostId,
    pub engine: Engine,
    pub name: String,
    pub driver: String,
    pub subnet: Option<String>,
    pub used_by: usize,
    pub removable: bool,
}

#[derive(Debug, Clone)]
pub struct VolumeRow {
    pub host: HostId,
    pub engine: Engine,
    pub name: String,
    pub kind: String,
    pub pool: Option<String>,
    pub size_label: String,
    pub used_by: usize,
    pub removable: bool,
}

#[derive(Debug, Clone)]
pub struct InstanceSnap {
    pub host: HostId,
    pub instance: String,
    pub name: String,
    pub created: Option<String>,
    pub stateful: bool,
}

#[derive(Debug, Clone)]
pub struct NetSpec {
    pub host: HostId,
    pub engine: Engine,
    pub name: String,
    pub driver: String,
}

#[derive(Debug, Clone)]
pub struct VolumeSpec {
    pub host: HostId,
    pub engine: Engine,
    pub name: String,
    pub pool: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapAction {
    Create,
    Delete,
    Restore,
}

pub const HEALTH_POINTS: usize = 60;

#[derive(Debug, Clone)]
pub struct HostHealth {
    pub host: HostId,
    pub cpu: Option<f64>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub disk_used: Option<u64>,
    pub disk_total: Option<u64>,
    pub cpu_hist: Vec<f64>,
    pub mem_hist: Vec<f64>,
    pub disk_hist: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckId {
    DockerPkg,
    DockerGroup,
    DockerDaemon,
    IncusPkg,
    IncusGroup,
    IncusDaemon,
    IncusInit,
    Vte,
}

impl CheckId {
    pub fn title(self) -> &'static str {
        match self {
            Self::DockerPkg => "Docker package",
            Self::DockerGroup => "Docker group",
            Self::DockerDaemon => "Docker daemon",
            Self::IncusPkg => "Incus package",
            Self::IncusGroup => "Incus group",
            Self::IncusDaemon => "Incus daemon",
            Self::IncusInit => "Incus storage",
            Self::Vte => "VTE terminal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Ok,
    Fail,
}

#[derive(Debug, Clone)]
pub struct DiagCheck {
    pub id: CheckId,
    pub level: CheckLevel,
    pub detail: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DiagReport {
    pub checks: Vec<DiagCheck>,
}

impl Snapshot {
    pub fn filtered(&self, host: HostFilter, kind: Option<Kind>) -> impl Iterator<Item = &Workload> {
        self.workloads.iter().filter(move |w| host.matches(w.host) && kind.is_none_or(|k| w.kind == k))
    }

    pub fn running(&self, host: HostFilter, kind: Option<Kind>) -> usize {
        self.filtered(host, kind).filter(|w| w.state == RunState::Running).count()
    }

    pub fn stopped(&self, host: HostFilter, kind: Option<Kind>) -> usize {
        self.filtered(host, kind).filter(|w| w.state == RunState::Stopped).count()
    }

    pub fn images(&self, host: HostFilter) -> impl Iterator<Item = &Image> {
        self.images.iter().filter(move |i| host.matches(i.host))
    }

    pub fn networks(&self, host: HostFilter) -> impl Iterator<Item = &NetRow> {
        self.networks.iter().filter(move |n| host.matches(n.host))
    }

    pub fn volumes(&self, host: HostFilter) -> impl Iterator<Item = &VolumeRow> {
        self.volumes.iter().filter(move |v| host.matches(v.host))
    }

    pub fn stacks(&self, host: HostFilter) -> Vec<Stack> {
        let mut map: std::collections::BTreeMap<(HostId, String), Stack> = std::collections::BTreeMap::new();
        for w in self.filtered(host, Some(Kind::Container)).filter(|w| w.engine == Engine::Docker) {
            let Some(project) = w.project.as_deref() else { continue };
            let slot = map.entry((w.host, project.to_string())).or_insert_with(|| Stack {
                host: w.host,
                name: project.to_string(),
                running: 0,
                total: 0,
                names: Vec::new(),
                project_dir: None,
                compose_file: None,
            });
            slot.total += 1;
            if w.state == RunState::Running {
                slot.running += 1;
            }
            slot.names.push(w.target_id().to_string());
            if slot.project_dir.is_none() {
                slot.project_dir = w.project_dir.clone();
            }
            if slot.compose_file.is_none() {
                slot.compose_file = w.compose_file.clone();
            }
        }
        map.into_values().collect()
    }
}

#[derive(Debug, Clone)]
pub struct Stack {
    pub host: HostId,
    pub name: String,
    pub running: usize,
    pub total: usize,
    pub names: Vec<String>,
    pub project_dir: Option<String>,
    pub compose_file: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_filter_matches_each_host() {
        assert!(HostFilter::All.matches(HostId::Local));
        assert!(HostFilter::All.matches(HostId::Remote));
        assert!(HostFilter::Local.matches(HostId::Local));
        assert!(!HostFilter::Local.matches(HostId::Remote));
    }

    #[test]
    fn run_state_maps_common_status_strings() {
        assert_eq!(RunState::from_status("running"), RunState::Running);
        assert_eq!(RunState::from_status("Exited (0)"), RunState::Stopped);
        assert_eq!(RunState::from_status("Stopped"), RunState::Stopped);
        assert_eq!(RunState::from_status("Paused"), RunState::Paused);
    }

    #[test]
    fn format_bytes_uses_binary_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn action_labels_are_stable() {
        assert_eq!(Action::Delete.gerund(), "Deleting");
        assert_eq!(Action::Start.done(), "Started");
    }

    #[test]
    fn log_filter_hides_stderr_when_stdout_selected() {
        let chunk = LogChunk { seq: 1, stream: LogStream::Stderr, text: "boom".into() };
        assert!(!log_visible(&chunk, LogFilter::Stdout, ""));
        assert!(log_visible(&chunk, LogFilter::Stderr, ""));
        assert!(log_visible(&chunk, LogFilter::All, "BOOM"));
        assert!(!log_visible(&chunk, LogFilter::All, "nope"));
    }

    #[test]
    fn check_titles_are_stable() {
        assert_eq!(CheckId::IncusDaemon.title(), "Incus daemon");
        assert_eq!(CheckId::DockerGroup.title(), "Docker group");
    }

    #[test]
    fn stacks_group_docker_compose_labels() {
        let snap = Snapshot {
            workloads: vec![
                Workload {
                    host: HostId::Local,
                    engine: Engine::Docker,
                    kind: Kind::Container,
                    name: "web".into(),
                    id: "web".into(),
                    status: "running".into(),
                    state: RunState::Running,
                    image: None,
                    project: Some("app".into()),
                    project_dir: Some("/srv/app".into()),
                    compose_file: Some("/srv/app/compose.yaml".into()),
                },
                Workload {
                    host: HostId::Local,
                    engine: Engine::Docker,
                    kind: Kind::Container,
                    name: "db".into(),
                    id: "db".into(),
                    status: "exited".into(),
                    state: RunState::Stopped,
                    image: None,
                    project: Some("app".into()),
                    project_dir: Some("/srv/app".into()),
                    compose_file: Some("/srv/app/compose.yaml".into()),
                },
            ],
            ..Snapshot::default()
        };
        let stacks = snap.stacks(HostFilter::All);
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].name, "app");
        assert_eq!(stacks[0].running, 1);
        assert_eq!(stacks[0].total, 2);
        assert_eq!(stacks[0].project_dir.as_deref(), Some("/srv/app"));
        assert_eq!(stacks[0].compose_file.as_deref(), Some("/srv/app/compose.yaml"));
    }

    #[test]
    fn compose_file_joins_relative_config() {
        assert_eq!(
            compose_file_from_labels(Some("/srv/app"), Some("compose.yaml")),
            Some("/srv/app/compose.yaml".into())
        );
        assert_eq!(
            compose_file_from_labels(Some("/srv/app"), Some("/etc/compose.yaml")),
            Some("/etc/compose.yaml".into())
        );
        assert_eq!(compose_file_from_labels(None, Some("compose.yaml")), None);
    }

    #[test]
    fn networks_filter_by_host() {
        let snap = Snapshot {
            networks: vec![
                NetRow {
                    host: HostId::Local,
                    engine: Engine::Docker,
                    name: "a".into(),
                    driver: "bridge".into(),
                    subnet: None,
                    used_by: 0,
                    removable: true,
                },
                NetRow {
                    host: HostId::Remote,
                    engine: Engine::Docker,
                    name: "b".into(),
                    driver: "bridge".into(),
                    subnet: None,
                    used_by: 0,
                    removable: true,
                },
            ],
            ..Snapshot::default()
        };
        assert_eq!(snap.networks(HostFilter::Local).count(), 1);
        assert_eq!(snap.networks(HostFilter::All).count(), 2);
    }
}
