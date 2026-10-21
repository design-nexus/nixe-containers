//! Incus REST API over a Unix socket or certificate-authenticated HTTPS.

use crate::models::{
    Action, ConnState, CreateSpec, Engine, EngineStatus, HostId, Image, ImageDetail, InstanceSnap, Kind, LogChunk, LogStream,
    NetRow, ResourceSample, RunState, TermEvent, TermKind, TrustKind, VolumeRow, Workload, WorkloadDetail,
};
use crate::prefs::RemoteProfile;
use crate::security::tls::{self, PinVerifier};
use bytes::Bytes;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector};
use rustls::ClientConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

type UnixHttp = Client<UnixConnector, Full<Bytes>>;

#[derive(Debug, Deserialize)]
struct IncusEnvelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    error: Option<String>,
    operation: Option<String>,
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct IncusInstance {
    name: String,
    status: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    config: HashMap<String, String>,
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IncusImage {
    fingerprint: String,
    size: Option<u64>,
    created_at: Option<String>,
    aliases: Option<Vec<IncusAlias>>,
    properties: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct IncusAlias {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IncusState {
    status: Option<String>,
    cpu: Option<IncusCpu>,
    memory: Option<IncusMemory>,
    network: Option<HashMap<String, IncusNet>>,
}

#[derive(Debug, Deserialize)]
struct IncusCpu {
    usage: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct IncusMemory {
    usage: Option<u64>,
    #[serde(rename = "usage_peak")]
    usage_peak: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct IncusNet {
    addresses: Option<Vec<IncusAddr>>,
}

#[derive(Debug, Deserialize)]
struct IncusAddr {
    family: Option<String>,
    address: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IncusNetwork {
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    managed: Option<bool>,
    #[serde(default)]
    config: HashMap<String, String>,
    #[serde(default)]
    used_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IncusPool {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IncusVolume {
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    pool: Option<String>,
    #[serde(default)]
    config: HashMap<String, String>,
    #[serde(default)]
    used_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IncusSnapshot {
    name: String,
    created_at: Option<String>,
    stateful: Option<bool>,
}

pub fn unix_candidates_from(config_yml: Option<&str>, runtime_dir: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(text) = config_yml
        && let Some(path) = parse_unix_addr(text)
    {
        out.push(path);
    }
    if let Some(runtime) = runtime_dir.filter(|s| !s.is_empty()) {
        out.push(PathBuf::from(runtime).join("incus/unix.socket"));
    }
    out.push(PathBuf::from("/var/lib/incus/unix.socket"));
    out.push(PathBuf::from("/run/incus/unix.socket"));
    let mut unique = Vec::new();
    for path in out {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

pub fn parse_unix_addr(config_yml: &str) -> Option<PathBuf> {
    for line in config_yml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("addr:") else { continue };
        let rest = rest.trim().trim_matches('"').trim_matches('\'');
        if rest == "unix://" {
            return None;
        }
        if let Some(path) = rest.strip_prefix("unix://")
            && !path.is_empty()
        {
            return Some(PathBuf::from(path));
        }
    }
    None
}

pub fn local_socket_candidates() -> Vec<PathBuf> {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".config"))
        .join("incus/config.yml");
    let text = std::fs::read_to_string(config).ok();
    unix_candidates_from(text.as_deref(), std::env::var("XDG_RUNTIME_DIR").ok().as_deref())
}

pub async fn probe_local() -> (EngineStatus, Vec<Workload>) {
    match local_socket().await {
        Ok(path) => match list_unix(&path).await {
            Ok((detail, workloads)) => (EngineStatus::connected(HostId::Local, Engine::Incus, detail), workloads),
            Err(err) => (EngineStatus::unavailable(HostId::Local, Engine::Incus, err), Vec::new()),
        },
        Err(status) => (status, Vec::new()),
    }
}

async fn local_socket() -> Result<PathBuf, EngineStatus> {
    let candidates = local_socket_candidates();
    let mut last_missing: Option<PathBuf> = None;
    let mut last_denied: Option<PathBuf> = None;
    for path in candidates {
        match std::os::unix::net::UnixStream::connect(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => last_missing = Some(path),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => last_denied = Some(path),
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Err(EngineStatus::unavailable(
                    HostId::Local,
                    Engine::Incus,
                    format!("Incus is not running (socket {} is closed).", path.display()),
                ));
            }
            Err(_) => last_missing = Some(path),
            Ok(_) => return Ok(path),
        }
    }
    if let Some(path) = last_denied {
        return Err(EngineStatus::permission(
            HostId::Local,
            Engine::Incus,
            format!("Incus's socket is at {} but this user cannot access it.", path.display()),
        ));
    }
    Err(EngineStatus::unavailable(
        HostId::Local,
        Engine::Incus,
        last_missing
            .map(|p| format!("Incus is not running. No usable socket (last tried {}).", p.display()))
            .unwrap_or_else(|| "Incus is not available on this computer.".into()),
    ))
}

async fn list_unix(path: &Path) -> Result<(String, Vec<Workload>), String> {
    let info: Value = unix_call(path, Method::GET, "/1.0", None).await?;
    let instances: Vec<IncusInstance> =
        unix_call(path, Method::GET, "/1.0/instances?recursion=1", None).await.unwrap_or_default();
    Ok((server_detail(&info), map_instances(HostId::Local, instances)))
}

fn server_detail(info: &Value) -> String {
    info.get("environment")
        .and_then(|e| e.get("server_version"))
        .and_then(|v| v.as_str())
        .map(|v| format!("Incus {v}"))
        .unwrap_or_else(|| "Incus".into())
}

fn map_instances(host: HostId, instances: Vec<IncusInstance>) -> Vec<Workload> {
    instances
        .into_iter()
        .map(|i| {
            let kind = if i.kind.contains("virtual") { Kind::VirtualMachine } else { Kind::Container };
            let image = i.config.get("image.description").cloned().or_else(|| {
                let os = i.config.get("image.os");
                let rel = i.config.get("image.release");
                match (os, rel) {
                    (Some(o), Some(r)) => Some(format!("{o}/{r}")),
                    (Some(o), None) => Some(o.clone()),
                    _ => None,
                }
            });
            Workload {
                host,
                engine: Engine::Incus,
                kind,
                name: i.name.clone(),
                id: i.name,
                state: RunState::from_status(&i.status),
                status: i.status,
                image,
                project: None,
                project_dir: None,
                compose_file: None,
            }
        })
        .collect()
}

pub async fn probe_remote(profile: &RemoteProfile) -> (EngineStatus, Vec<Workload>) {
    if !profile.incus_enabled || profile.incus_url.trim().is_empty() {
        return (EngineStatus { host: HostId::Remote, engine: Engine::Incus, state: ConnState::Unconfigured }, Vec::new());
    }
    match list_https(profile).await {
        Ok((detail, workloads)) => (EngineStatus::connected(HostId::Remote, Engine::Incus, detail), workloads),
        Err(RemoteError::Untrusted(fp)) => (
            EngineStatus {
                host: HostId::Remote,
                engine: Engine::Incus,
                state: ConnState::Untrusted { fingerprint: fp, kind: TrustKind::Tls },
            },
            Vec::new(),
        ),
        Err(RemoteError::Other(err)) => (EngineStatus::unavailable(HostId::Remote, Engine::Incus, err), Vec::new()),
    }
}

enum RemoteError {
    Untrusted(String),
    Other(String),
}

impl From<RemoteError> for String {
    fn from(err: RemoteError) -> Self {
        match err {
            RemoteError::Untrusted(fp) => format!("Unknown TLS identity. Fingerprint {fp}"),
            RemoteError::Other(e) => e,
        }
    }
}

async fn list_https(profile: &RemoteProfile) -> Result<(String, Vec<Workload>), RemoteError> {
    let (client, url, pin) = https_client(profile)?;
    let info: Value = https_call(&client, &url, &pin, Method::GET, "/1.0", None).await?;
    let instances: Vec<IncusInstance> =
        https_call(&client, &url, &pin, Method::GET, "/1.0/instances?recursion=1", None).await.unwrap_or_else(|_| Vec::new());
    Ok((server_detail(&info), map_instances(HostId::Remote, instances)))
}

fn tls_setup(profile: &RemoteProfile) -> Result<(ClientConfig, String, Arc<PinVerifier>), RemoteError> {
    let url = profile.incus_url.trim().trim_end_matches('/');
    if !url.starts_with("https://") {
        return Err(RemoteError::Other("Incus remotes must use HTTPS.".into()));
    }
    if profile.incus_client_cert.trim().is_empty() || profile.incus_client_key.trim().is_empty() {
        return Err(RemoteError::Other("Incus HTTPS needs a client certificate and key.".into()));
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pin = PinVerifier::new(Some(profile.incus_server_fingerprint.clone()).filter(|s| !s.is_empty()));
    let certs = load_certs(Path::new(&profile.incus_client_cert))?;
    let key = load_key(Path::new(&profile.incus_client_key))?;
    let mut tls = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(pin.clone())
        .with_client_auth_cert(certs, key)
        .map_err(|e| RemoteError::Other(e.to_string()))?;
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((tls, url.to_string(), pin))
}

fn https_client(profile: &RemoteProfile) -> Result<(reqwest::Client, String, Arc<PinVerifier>), RemoteError> {
    let (tls, url, pin) = tls_setup(profile)?;
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .user_agent("nixe-containers")
        .build()
        .map_err(|e| RemoteError::Other(e.to_string()))?;
    Ok((client, url, pin))
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, RemoteError> {
    let data = std::fs::read(path).map_err(|e| RemoteError::Other(format!("{}: {e}", path.display())))?;
    rustls_pemfile::certs(&mut data.as_slice()).collect::<Result<Vec<_>, _>>().map_err(|e| RemoteError::Other(e.to_string()))
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, RemoteError> {
    let data = std::fs::read(path).map_err(|e| RemoteError::Other(format!("{}: {e}", path.display())))?;
    rustls_pemfile::private_key(&mut data.as_slice())
        .map_err(|e| RemoteError::Other(e.to_string()))?
        .ok_or_else(|| RemoteError::Other(format!("no private key in {}", path.display())))
}

async fn unix_call<T: serde::de::DeserializeOwned>(
    path: &Path,
    method: Method,
    api_path: &str,
    body: Option<Value>,
) -> Result<T, String> {
    let env = unix_envelope(path, method, api_path, body, true).await?;
    parse_metadata(env)
}

async fn unix_envelope(
    path: &Path,
    method: Method,
    api_path: &str,
    body: Option<Value>,
    wait: bool,
) -> Result<IncusEnvelope, String> {
    let client: UnixHttp = Client::unix();
    let payload = body.map(|v| Bytes::from(v.to_string())).unwrap_or_default();
    let uri = hyperlocal::Uri::new(path, api_path);
    let mut builder = Request::builder().method(method.clone()).uri(uri).header("User-Agent", "nixe-containers");
    if !payload.is_empty() {
        builder = builder.header("Content-Type", "application/json");
    }
    let req = builder.body(Full::new(payload)).map_err(|e| e.to_string())?;
    let res = client.request(req).await.map_err(|e| e.to_string())?;
    let bytes = res.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
    let env: IncusEnvelope = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if wait { Box::pin(finish_unix(path, env)).await } else { check_env(env) }
}

async fn finish_unix(path: &Path, env: IncusEnvelope) -> Result<IncusEnvelope, String> {
    let env = check_env(env)?;
    if env.kind.as_deref() == Some("async") {
        let id = operation_id(&env).ok_or_else(|| "Incus started an operation without an id".to_string())?;
        return unix_envelope(path, Method::GET, &format!("/1.0/operations/{id}/wait?timeout=300"), None, true).await;
    }
    Ok(env)
}

async fn https_call<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    pin: &Arc<PinVerifier>,
    method: Method,
    api_path: &str,
    body: Option<Value>,
) -> Result<T, RemoteError> {
    let env = https_envelope(client, url, pin, method, api_path, body, true).await?;
    parse_metadata(env).map_err(RemoteError::Other)
}

async fn https_envelope(
    client: &reqwest::Client,
    url: &str,
    pin: &Arc<PinVerifier>,
    method: Method,
    api_path: &str,
    body: Option<Value>,
    wait: bool,
) -> Result<IncusEnvelope, RemoteError> {
    let req_url = format!("{url}{api_path}");
    let mut req = client.request(method.clone(), &req_url);
    if let Some(v) = &body {
        req = req.json(v);
    }
    let res = match req.send().await {
        Ok(r) => r,
        Err(err) => return Err(map_tls_error(err.to_string(), pin)),
    };
    let env: IncusEnvelope = res.json().await.map_err(|e| RemoteError::Other(e.to_string()))?;
    if wait { Box::pin(finish_https(client, url, pin, env)).await } else { check_env(env).map_err(RemoteError::Other) }
}

async fn finish_https(
    client: &reqwest::Client,
    url: &str,
    pin: &Arc<PinVerifier>,
    env: IncusEnvelope,
) -> Result<IncusEnvelope, RemoteError> {
    let env = check_env(env).map_err(RemoteError::Other)?;
    if env.kind.as_deref() == Some("async") {
        let id = operation_id(&env).ok_or_else(|| RemoteError::Other("Incus started an operation without an id".into()))?;
        return https_envelope(client, url, pin, Method::GET, &format!("/1.0/operations/{id}/wait?timeout=300"), None, true)
            .await;
    }
    Ok(env)
}

fn check_env(env: IncusEnvelope) -> Result<IncusEnvelope, String> {
    if env.kind.as_deref() == Some("error") {
        return Err(env.error.filter(|s| !s.is_empty()).unwrap_or_else(|| "Incus returned an error".into()));
    }
    Ok(env)
}

fn operation_id(env: &IncusEnvelope) -> Option<String> {
    env.metadata
        .as_ref()
        .and_then(|m| m.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| env.operation.as_ref().and_then(|op| op.rsplit('/').next()).filter(|s| !s.is_empty()).map(str::to_string))
}

fn parse_metadata<T: serde::de::DeserializeOwned>(env: IncusEnvelope) -> Result<T, String> {
    if let Some(err) = env.error.filter(|s| !s.is_empty()) {
        return Err(err);
    }
    let meta = env.metadata.unwrap_or(Value::Null);
    if meta.get("err").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()) {
        return Err(meta["err"].as_str().unwrap_or("operation failed").into());
    }
    serde_json::from_value(meta).map_err(|e| e.to_string())
}

fn map_tls_error(msg: String, pin: &Arc<PinVerifier>) -> RemoteError {
    if msg.contains("fingerprint does not match") {
        return RemoteError::Other("The Incus certificate does not match the stored fingerprint.".into());
    }
    if let Some(fp) = tls::untrusted_fingerprint(&msg).or_else(|| pin.seen()) {
        return RemoteError::Untrusted(fp);
    }
    RemoteError::Other(msg)
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

enum Conn {
    Unix(PathBuf),
    Https { client: reqwest::Client, url: String, pin: Arc<PinVerifier> },
}

async fn connect(host: HostId, remote: &RemoteProfile) -> Result<Conn, String> {
    match host {
        HostId::Local => local_socket().await.map(Conn::Unix).map_err(|st| match st.state {
            ConnState::Unavailable { reason } | ConnState::Permission { reason } | ConnState::Insecure { reason } => reason,
            _ => "Incus is not available on this computer.".into(),
        }),
        HostId::Remote => {
            let (client, url, pin) = https_client(remote).map_err(String::from)?;
            Ok(Conn::Https { client, url, pin })
        }
    }
}

async fn call<T: serde::de::DeserializeOwned>(
    conn: &Conn,
    method: Method,
    api_path: &str,
    body: Option<Value>,
) -> Result<T, String> {
    match conn {
        Conn::Unix(path) => unix_call(path, method, api_path, body).await,
        Conn::Https { client, url, pin } => https_call(client, url, pin, method, api_path, body).await.map_err(String::from),
    }
}

pub async fn operate(host: HostId, remote: &RemoteProfile, name: &str, action: Action) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let verb = match action {
        Action::Start => "start",
        Action::Stop => "stop",
        Action::Restart => "restart",
        Action::Delete => {
            let path = format!("/1.0/instances/{}?force=1", enc(name));
            let _: Value = call(&conn, Method::DELETE, &path, None).await?;
            return Ok(format!("Deleted {name}"));
        }
    };
    let body = json!({ "action": verb, "timeout": 30, "force": action == Action::Stop, "stateful": false });
    let _: Value = call(&conn, Method::PUT, &format!("/1.0/instances/{}/state", enc(name)), Some(body)).await?;
    Ok(format!("{} {name}", action.done()))
}

pub async fn create(remote: &RemoteProfile, spec: &CreateSpec) -> Result<String, String> {
    let conn = connect(spec.host, remote).await?;
    let kind = if spec.kind == Kind::VirtualMachine { "virtual-machine" } else { "container" };
    let source = image_source(&spec.image);
    let body = json!({ "name": spec.name, "type": kind, "source": source });
    let _: Value = call(&conn, Method::POST, "/1.0/instances", Some(body)).await?;
    if spec.start {
        let start = json!({ "action": "start", "timeout": 30, "force": false, "stateful": false });
        let _: Value = call(&conn, Method::PUT, &format!("/1.0/instances/{}/state", enc(&spec.name)), Some(start)).await?;
        return Ok(format!("Created and started {}", spec.name));
    }
    Ok(format!("Created {}", spec.name))
}

fn image_source(image: &str) -> Value {
    let image = image.trim();
    if image.chars().all(|c| c.is_ascii_hexdigit()) && image.len() >= 12 {
        return json!({ "type": "image", "fingerprint": image });
    }
    json!({ "type": "image", "alias": image })
}

pub async fn inspect(host: HostId, remote: &RemoteProfile, name: &str) -> Result<WorkloadDetail, String> {
    let conn = connect(host, remote).await?;
    let instance: IncusInstance = call(&conn, Method::GET, &format!("/1.0/instances/{}", enc(name)), None).await?;
    let state: IncusState = call(&conn, Method::GET, &format!("/1.0/instances/{}/state", enc(name)), None)
        .await
        .unwrap_or(IncusState { status: None, cpu: None, memory: None, network: None });
    let status = state.status.clone().unwrap_or(instance.status.clone());
    let kind = if instance.kind.contains("virtual") { Kind::VirtualMachine } else { Kind::Container };
    let ip = state.network.as_ref().and_then(|nets| {
        nets.values().flat_map(|n| n.addresses.as_deref().unwrap_or(&[])).find_map(|a| {
            if a.family.as_deref() == Some("inet") && a.scope.as_deref() != Some("link") { a.address.clone() } else { None }
        })
    });
    let image = instance.config.get("image.description").cloned();
    let mut extra = Vec::new();
    extra.push(("Type".into(), instance.kind.clone()));
    if let Some(arch) = instance.config.get("image.architecture") {
        extra.push(("Architecture".into(), arch.clone()));
    }
    let mut detail = WorkloadDetail {
        host,
        engine: Engine::Incus,
        kind,
        name: instance.name,
        status: status.clone(),
        state: RunState::from_status(&status),
        image,
        created: instance.created_at,
        command: None,
        ip,
        cpu: None,
        memory_used: state.memory.as_ref().and_then(|m| m.usage),
        memory_limit: state.memory.as_ref().and_then(|m| m.usage_peak),
        extra,
        snapshots: list_snapshots_on(&conn, host, name).await.unwrap_or_default(),
    };
    if detail.state == RunState::Running
        && let Ok(sample) = stats_on(&conn, host, name, 0).await
    {
        detail.cpu = sample.cpu;
        detail.memory_used = sample.memory_used.or(detail.memory_used);
        detail.memory_limit = sample.memory_limit.or(detail.memory_limit);
    }
    Ok(detail)
}

pub async fn stats(host: HostId, remote: &RemoteProfile, name: &str, seq: u64) -> Result<ResourceSample, String> {
    let conn = connect(host, remote).await?;
    stats_on(&conn, host, name, seq).await
}

async fn stats_on(conn: &Conn, host: HostId, name: &str, seq: u64) -> Result<ResourceSample, String> {
    let s1: IncusState = call(conn, Method::GET, &format!("/1.0/instances/{}/state", enc(name)), None).await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let s2: IncusState = call(conn, Method::GET, &format!("/1.0/instances/{}/state", enc(name)), None).await?;
    let cpu = match (s1.cpu.and_then(|c| c.usage), s2.cpu.and_then(|c| c.usage)) {
        (Some(a), Some(b)) if b >= a => Some((b.saturating_sub(a) as f64 / 400_000_000.0) * 100.0),
        _ => None,
    };
    Ok(ResourceSample {
        host,
        engine: Engine::Incus,
        name: name.into(),
        cpu,
        memory_used: s2.memory.as_ref().and_then(|m| m.usage),
        memory_limit: s2.memory.as_ref().and_then(|m| m.usage_peak),
        seq,
    })
}

pub async fn list_images(host: HostId, remote: &RemoteProfile) -> Result<Vec<Image>, String> {
    let conn = connect(host, remote).await?;
    let images: Vec<IncusImage> = call(&conn, Method::GET, "/1.0/images?recursion=1", None).await.unwrap_or_default();
    Ok(images.into_iter().map(|img| map_image(host, img)).collect())
}

fn map_image(host: HostId, img: IncusImage) -> Image {
    let name = img
        .aliases
        .as_ref()
        .and_then(|a| a.first())
        .map(|a| a.name.clone())
        .or_else(|| img.properties.as_ref().and_then(|p| p.get("description").cloned()))
        .unwrap_or_else(|| img.fingerprint.chars().take(12).collect());
    Image { host, engine: Engine::Incus, id: img.fingerprint, name, size: img.size.unwrap_or(0), created: img.created_at }
}

pub async fn pull_image(host: HostId, remote: &RemoteProfile, reference: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let body = json!({
        "source": {
            "type": "image",
            "mode": "pull",
            "server": "https://images.linuxcontainers.org",
            "protocol": "simplestreams",
            "alias": reference.trim(),
        }
    });
    let _: Value = call(&conn, Method::POST, "/1.0/images", Some(body)).await?;
    Ok(format!("Pulled {reference}"))
}

pub async fn host_resources(
    host: HostId,
    remote: &RemoteProfile,
) -> Result<(Option<u64>, Option<u64>, Option<u64>, Option<u64>), String> {
    let conn = connect(host, remote).await?;
    let raw: Value = call(&conn, Method::GET, "/1.0/resources", None).await.unwrap_or(Value::Null);
    let mem_used = raw.pointer("/memory/used").and_then(|v| v.as_u64());
    let mem_total = raw.pointer("/memory/total").and_then(|v| v.as_u64());
    let mut disk_used = raw.pointer("/storage/space/used").and_then(|v| v.as_u64());
    let mut disk_total = raw.pointer("/storage/space/total").and_then(|v| v.as_u64());
    if disk_used.is_none() || disk_total.is_none() {
        let pools: Vec<Value> = call(&conn, Method::GET, "/1.0/storage-pools?recursion=1", None).await.unwrap_or_default();
        let mut used = 0u64;
        let mut total = 0u64;
        for pool in pools {
            let name = pool.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let res: Value = call(&conn, Method::GET, &format!("/1.0/storage-pools/{}/resources", enc(name)), None)
                .await
                .unwrap_or(Value::Null);
            used += res.pointer("/space/used").and_then(|v| v.as_u64()).unwrap_or(0);
            total += res.pointer("/space/total").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        if disk_used.is_none() && used > 0 {
            disk_used = Some(used);
        }
        if disk_total.is_none() && total > 0 {
            disk_total = Some(total);
        }
    }
    Ok((mem_used, mem_total, disk_used, disk_total))
}

pub async fn storage_ready(host: HostId, remote: &RemoteProfile) -> bool {
    let Ok(conn) = connect(host, remote).await else { return false };
    let pools: Vec<Value> = call(&conn, Method::GET, "/1.0/storage-pools", None).await.unwrap_or_default();
    !pools.is_empty()
}

pub async fn inspect_image(host: HostId, remote: &RemoteProfile, id: &str) -> Result<ImageDetail, String> {
    let conn = connect(host, remote).await?;
    let img: IncusImage = call(&conn, Method::GET, &format!("/1.0/images/{}", enc(id)), None).await?;
    let mut extra = Vec::new();
    extra.push(("Fingerprint".into(), img.fingerprint.clone()));
    if let Some(props) = &img.properties {
        for key in ["os", "release", "architecture", "description"] {
            if let Some(v) = props.get(key) {
                extra.push((key.to_string(), v.clone()));
            }
        }
    }
    if let Some(aliases) = &img.aliases {
        let names: Vec<_> = aliases.iter().map(|a| a.name.clone()).collect();
        if !names.is_empty() {
            extra.push(("Aliases".into(), names.join(", ")));
        }
    }
    Ok(ImageDetail { image: map_image(host, img), extra })
}

pub async fn remove_image(host: HostId, remote: &RemoteProfile, id: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let _: Value = call(&conn, Method::DELETE, &format!("/1.0/images/{}", enc(id)), None).await?;
    Ok(format!("Removed {id}"))
}

pub async fn list_networks(host: HostId, remote: &RemoteProfile) -> Result<Vec<NetRow>, String> {
    let conn = connect(host, remote).await?;
    let nets: Vec<IncusNetwork> = call(&conn, Method::GET, "/1.0/networks?recursion=1", None).await.unwrap_or_default();
    Ok(nets
        .into_iter()
        .map(|n| {
            let managed = n.managed.unwrap_or(false);
            NetRow {
                host,
                engine: Engine::Incus,
                name: n.name,
                driver: n.kind.unwrap_or_default(),
                subnet: n.config.get("ipv4.address").cloned().filter(|s| !s.is_empty() && s != "none"),
                used_by: n.used_by.len(),
                removable: managed,
            }
        })
        .collect())
}

pub async fn create_network(host: HostId, remote: &RemoteProfile, name: &str, driver: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let kind = if driver.trim().is_empty() { "bridge" } else { driver.trim() };
    let body = json!({ "name": name, "type": kind });
    let _: Value = call(&conn, Method::POST, "/1.0/networks", Some(body)).await?;
    Ok(format!("Created {name}"))
}

pub async fn remove_network(host: HostId, remote: &RemoteProfile, name: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let net: IncusNetwork = call(&conn, Method::GET, &format!("/1.0/networks/{}", enc(name)), None).await?;
    if !net.managed.unwrap_or(false) {
        return Err(format!("Cannot remove unmanaged Incus network {name}."));
    }
    let _: Value = call(&conn, Method::DELETE, &format!("/1.0/networks/{}", enc(name)), None).await?;
    Ok(format!("Removed {name}"))
}

pub async fn list_volumes(host: HostId, remote: &RemoteProfile) -> Result<Vec<VolumeRow>, String> {
    let conn = connect(host, remote).await?;
    let pools: Vec<IncusPool> = call(&conn, Method::GET, "/1.0/storage-pools?recursion=1", None).await.unwrap_or_default();
    let mut out = Vec::new();
    for pool in pools {
        let path = format!("/1.0/storage-pools/{}/volumes?recursion=1", enc(&pool.name));
        let vols: Vec<IncusVolume> = call(&conn, Method::GET, &path, None).await.unwrap_or_default();
        for v in vols {
            let kind = v.kind.unwrap_or_else(|| "custom".into());
            let pool_name = v.pool.clone().unwrap_or_else(|| pool.name.clone());
            out.push(VolumeRow {
                host,
                engine: Engine::Incus,
                name: v.name,
                kind: kind.clone(),
                pool: Some(pool_name),
                size_label: v.config.get("size").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into()),
                used_by: v.used_by.len(),
                removable: kind == "custom",
            });
        }
    }
    Ok(out)
}

pub async fn create_volume(host: HostId, remote: &RemoteProfile, name: &str, pool: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let pool = if pool.trim().is_empty() { "default" } else { pool.trim() };
    let body = json!({ "name": name, "type": "custom" });
    let _: Value = call(&conn, Method::POST, &format!("/1.0/storage-pools/{}/volumes", enc(pool)), Some(body)).await?;
    Ok(format!("Created {name} on {pool}"))
}

pub async fn remove_volume(host: HostId, remote: &RemoteProfile, name: &str, pool: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let pool = if pool.trim().is_empty() { "default" } else { pool.trim() };
    let path = format!("/1.0/storage-pools/{}/volumes/custom/{}", enc(pool), enc(name));
    let _: Value = call(&conn, Method::DELETE, &path, None).await?;
    Ok(format!("Removed {name}"))
}

pub async fn list_snapshots(host: HostId, remote: &RemoteProfile, instance: &str) -> Result<Vec<InstanceSnap>, String> {
    let conn = connect(host, remote).await?;
    list_snapshots_on(&conn, host, instance).await
}

async fn list_snapshots_on(conn: &Conn, host: HostId, instance: &str) -> Result<Vec<InstanceSnap>, String> {
    let snaps: Vec<IncusSnapshot> =
        call(conn, Method::GET, &format!("/1.0/instances/{}/snapshots?recursion=1", enc(instance)), None)
            .await
            .unwrap_or_default();
    Ok(snaps
        .into_iter()
        .map(|s| InstanceSnap {
            host,
            instance: instance.to_string(),
            name: s.name,
            created: s.created_at,
            stateful: s.stateful.unwrap_or(false),
        })
        .collect())
}

pub async fn create_snapshot(host: HostId, remote: &RemoteProfile, instance: &str, snap: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let body = json!({ "name": snap, "stateful": false });
    let _: Value = call(&conn, Method::POST, &format!("/1.0/instances/{}/snapshots", enc(instance)), Some(body)).await?;
    Ok(format!("Created snapshot {snap}"))
}

pub async fn delete_snapshot(host: HostId, remote: &RemoteProfile, instance: &str, snap: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let _: Value =
        call(&conn, Method::DELETE, &format!("/1.0/instances/{}/snapshots/{}", enc(instance), enc(snap)), None).await?;
    Ok(format!("Deleted snapshot {snap}"))
}

pub async fn restore_snapshot(host: HostId, remote: &RemoteProfile, instance: &str, snap: &str) -> Result<String, String> {
    let conn = connect(host, remote).await?;
    let body = json!({ "restore": snap });
    let _: Value = call(&conn, Method::PUT, &format!("/1.0/instances/{}", enc(instance)), Some(body)).await?;
    Ok(format!("Restored {instance} to {snap}"))
}

async fn start_operation(conn: &Conn, method: Method, api_path: &str, body: Option<Value>) -> Result<Value, String> {
    let env = match conn {
        Conn::Unix(path) => unix_envelope(path, method, api_path, body, false).await?,
        Conn::Https { client, url, pin } => {
            https_envelope(client, url, pin, method, api_path, body, false).await.map_err(String::from)?
        }
    };
    env.metadata.ok_or_else(|| "Incus returned no operation metadata".into())
}

fn op_id_and_fds(meta: &Value) -> Result<(String, HashMap<String, String>), String> {
    let id = meta.get("id").and_then(|v| v.as_str()).ok_or_else(|| "Incus operation has no id".to_string())?.to_string();
    let fds = meta
        .get("metadata")
        .and_then(|m| m.get("fds"))
        .and_then(|v| v.as_object())
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    Ok((id, fds))
}

async fn read_text(conn: &Conn, api_path: &str) -> Result<String, String> {
    match conn {
        Conn::Unix(path) => unix_text(path, api_path).await,
        Conn::Https { client, url, pin } => https_text(client, url, pin, api_path).await,
    }
}

async fn unix_text(path: &Path, api_path: &str) -> Result<String, String> {
    let client: UnixHttp = Client::unix();
    let uri = hyperlocal::Uri::new(path, api_path);
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("User-Agent", "nixe-containers")
        .body(Full::new(Bytes::new()))
        .map_err(|e| e.to_string())?;
    let res = client.request(req).await.map_err(|e| e.to_string())?;
    let bytes = res.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
    if let Ok(env) = serde_json::from_slice::<IncusEnvelope>(&bytes) {
        if let Some(err) = env.error.filter(|s| !s.is_empty()) {
            return Err(err);
        }
        if let Some(Value::String(s)) = env.metadata {
            return Ok(s);
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn https_text(client: &reqwest::Client, url: &str, pin: &Arc<PinVerifier>, api_path: &str) -> Result<String, String> {
    let res = match client.get(format!("{url}{api_path}")).send().await {
        Ok(r) => r,
        Err(err) => return Err(map_tls_error(err.to_string(), pin).into()),
    };
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    if let Ok(env) = serde_json::from_slice::<IncusEnvelope>(&bytes) {
        if let Some(err) = env.error.filter(|s| !s.is_empty()) {
            return Err(err);
        }
        if let Some(Value::String(s)) = env.metadata {
            return Ok(s);
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn collect_logs(conn: &Conn, name: &str) -> Result<String, String> {
    let files: Vec<String> =
        call(conn, Method::GET, &format!("/1.0/instances/{}/logs", enc(name)), None).await.unwrap_or_default();
    if files.is_empty() {
        return read_text(conn, &format!("/1.0/instances/{}/console", enc(name))).await.or_else(|_| Ok(String::new()));
    }
    let mut out = String::new();
    for file in files {
        let body = read_text(conn, &format!("/1.0/instances/{}/logs/{}", enc(name), enc(&file))).await.unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("== {file} ==\n"));
        out.push_str(&body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
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
    let conn = connect(host, remote).await?;
    let mut last = String::new();
    loop {
        let text = collect_logs(&conn, name).await?;
        if text != last {
            let suffix = if last.is_empty() { text.clone() } else { text.strip_prefix(&last).unwrap_or(&text).to_string() };
            last = text;
            if !suffix.is_empty() && chunks.send(LogChunk { seq, stream: LogStream::Console, text: suffix }).is_err() {
                break;
            }
        }
        if !follow {
            break;
        }
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
    Ok(())
}

pub async fn session(
    host: HostId,
    remote: &RemoteProfile,
    name: &str,
    kind: TermKind,
    events: mpsc::UnboundedReceiver<TermEvent>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let conn = connect(host, remote).await?;
    let body = match kind {
        TermKind::Exec => json!({
            "command": ["/bin/sh", "-c", "exec bash || exec sh"],
            "wait-for-websocket": true,
            "interactive": true,
            "environment": { "TERM": "xterm-256color" },
            "width": 80,
            "height": 24
        }),
        TermKind::Console => json!({
            "type": "console",
            "width": 80,
            "height": 24
        }),
    };
    let path = match kind {
        TermKind::Exec => format!("/1.0/instances/{}/exec", enc(name)),
        TermKind::Console => format!("/1.0/instances/{}/console", enc(name)),
    };
    let meta = start_operation(&conn, Method::POST, &path, Some(body)).await?;
    let (op_id, fds) = op_id_and_fds(&meta)?;
    let data_secret = fds.get("0").cloned().ok_or_else(|| "Incus did not return a data websocket".to_string())?;
    let control_secret = fds.get("control").cloned();
    let data_path = format!("/1.0/operations/{}/websocket?secret={}", enc(&op_id), enc(&data_secret));
    run_ws(&conn, remote, &data_path, control_secret.as_deref().map(|s| (op_id.as_str(), s)), events, out, cancel).await
}

async fn run_ws(
    conn: &Conn,
    remote: &RemoteProfile,
    data_path: &str,
    control: Option<(&str, &str)>,
    mut events: mpsc::UnboundedReceiver<TermEvent>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    match conn {
        Conn::Unix(path) => {
            let mut data = unix_ws(path, data_path).await?;
            let mut control_ws = match control {
                Some((id, secret)) => {
                    Some(unix_ws(path, &format!("/1.0/operations/{}/websocket?secret={}", enc(id), enc(secret))).await?)
                }
                None => None,
            };
            pump_ws(&mut data, control_ws.as_mut(), &mut events, &out, &mut cancel).await
        }
        Conn::Https { url, .. } => {
            let tls = tls_setup(remote).map_err(String::from)?.0;
            let mut data = https_ws(url, &tls, data_path).await?;
            let mut control_ws = match control {
                Some((id, secret)) => {
                    Some(https_ws(url, &tls, &format!("/1.0/operations/{}/websocket?secret={}", enc(id), enc(secret))).await?)
                }
                None => None,
            };
            pump_ws(&mut data, control_ws.as_mut(), &mut events, &out, &mut cancel).await
        }
    }
}

async fn unix_ws(path: &Path, api_path: &str) -> Result<tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>, String> {
    let stream = tokio::net::UnixStream::connect(path).await.map_err(|e| e.to_string())?;
    let req = format!("ws://incus{api_path}").into_client_request().map_err(|e| e.to_string())?;
    let (ws, _) = tokio_tungstenite::client_async(req, stream).await.map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn https_ws(
    url: &str,
    tls: &ClientConfig,
    api_path: &str,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    let ws_url = format!("{}{api_path}", url.replacen("https://", "wss://", 1));
    let req = ws_url.into_client_request().map_err(|e| e.to_string())?;
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls.clone()));
    let (ws, _) =
        tokio_tungstenite::connect_async_tls_with_config(req, None, false, Some(connector)).await.map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn pump_ws<S>(
    data: &mut S,
    mut control: Option<&mut S>,
    events: &mut mpsc::UnboundedReceiver<TermEvent>,
    out: &mpsc::UnboundedSender<Vec<u8>>,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(), String>
where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
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
                        if data.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(TermEvent::Resize { cols, rows }) => {
                        if let Some(ctl) = control.as_mut() {
                            let msg = json!({
                                "command": "window-resize",
                                "args": { "width": cols.to_string(), "height": rows.to_string() }
                            });
                            let _ = ctl.send(Message::Text(msg.to_string().into())).await;
                        }
                    }
                    Some(TermEvent::Close) | None => break,
                }
            }
            item = data.next() => {
                let Some(item) = item else { break };
                match item.map_err(|e| e.to_string())? {
                    Message::Binary(b) => {
                        if out.send(b.to_vec()).is_err() { break; }
                    }
                    Message::Text(t) => {
                        if out.send(t.as_bytes().to_vec()).is_err() { break; }
                    }
                    Message::Close(_) => break,
                    _ => {}
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
    fn default_unix_addr_does_not_invent_a_path() {
        let yml = "default-remote: local\nremotes:\n  local:\n    addr: unix://\n";
        assert_eq!(parse_unix_addr(yml), None);
        let paths = unix_candidates_from(Some(yml), Some("/run/user/1000"));
        assert_eq!(paths[0], PathBuf::from("/run/user/1000/incus/unix.socket"));
    }

    #[test]
    fn explicit_unix_path_is_used() {
        let yml = "addr: unix:///tmp/incus.socket\n";
        assert_eq!(parse_unix_addr(yml), Some(PathBuf::from("/tmp/incus.socket")));
    }

    #[test]
    fn path_encoding_keeps_safe_names() {
        assert_eq!(enc("web"), "web");
        assert_eq!(enc("a b"), "a%20b");
    }

    #[test]
    fn fingerprint_source_uses_hex() {
        let src = image_source("0123456789abcdef");
        assert_eq!(src["fingerprint"], "0123456789abcdef");
        let alias = image_source("ubuntu/24.04");
        assert_eq!(alias["alias"], "ubuntu/24.04");
    }
}
