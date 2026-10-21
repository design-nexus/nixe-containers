//! Engine I/O off the GTK thread. One Tokio runtime owns the work. Results
//! hop back to the main loop with `idle_add_once`.

use crate::models::{Action, CheckId, CreateSpec, Engine, HostId, NetSpec, SnapAction, TermEvent, TermKind, VolumeSpec};
use crate::prefs::RemoteProfile;
use crate::services::{connections, diagnose, host, inventory, ops};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{mpsc, watch};

pub enum Job {
    Refresh,
    TestRemote(Box<RemoteProfile>),
    Operate { host: HostId, engine: Engine, name: String, action: Action },
    Inspect { host: HostId, engine: Engine, name: String },
    Stats { host: HostId, engine: Engine, name: String, seq: u64 },
    Create(Box<CreateSpec>),
    PullImage { host: HostId, engine: Engine, reference: String },
    InspectImage { host: HostId, engine: Engine, id: String },
    RemoveImage { host: HostId, engine: Engine, id: String },
    FollowLogs { host: HostId, engine: Engine, names: Vec<String>, seq: u64, follow: bool },
    StackOp { host: HostId, names: Vec<String>, action: Action },
    StopLogs { seq: u64 },
    StartTerm { host: HostId, engine: Engine, name: String, seq: u64, kind: TermKind },
    TermInput { seq: u64, data: Vec<u8> },
    TermResize { seq: u64, cols: u16, rows: u16 },
    StopTerm { seq: u64 },
    Diagnose,
    Fix(CheckId),
    CreateNet(Box<NetSpec>),
    RemoveNet { host: HostId, engine: Engine, name: String },
    CreateVol(Box<VolumeSpec>),
    RemoveVol { host: HostId, engine: Engine, name: String, pool: Option<String> },
    SnapOp { host: HostId, instance: String, snap: String, action: SnapAction },
    Shutdown,
}

struct Live {
    logs: HashMap<u64, watch::Sender<bool>>,
    terms: HashMap<u64, mpsc::UnboundedSender<TermEvent>>,
    term_stop: HashMap<u64, watch::Sender<bool>>,
}

static TX: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();

pub fn start() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let runtime =
            tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("containers worker");
        runtime.block_on(async move {
            let live = Arc::new(Mutex::new(Live { logs: HashMap::new(), terms: HashMap::new(), term_stop: HashMap::new() }));
            let mut sampler = host::Sampler::default();
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(8));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut health = tokio::time::interval(std::time::Duration::from_secs(2));
            health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    job = rx.recv() => {
                        match job {
                            Some(Job::Shutdown) | None => break,
                            Some(Job::Refresh) => refresh_now().await,
                            Some(Job::TestRemote(profile)) => test_now(*profile).await,
                            Some(Job::Operate { host, engine, name, action }) => operate_now(host, engine, name, action).await,
                            Some(Job::Inspect { host, engine, name }) => inspect_now(host, engine, name).await,
                            Some(Job::Stats { host, engine, name, seq }) => stats_now(host, engine, name, seq).await,
                            Some(Job::Create(spec)) => create_now(*spec).await,
                            Some(Job::PullImage { host, engine, reference }) => pull_now(host, engine, reference).await,
                            Some(Job::InspectImage { host, engine, id }) => inspect_image_now(host, engine, id).await,
                            Some(Job::RemoveImage { host, engine, id }) => remove_image_now(host, engine, id).await,
                            Some(Job::FollowLogs { host, engine, names, seq, follow }) => {
                                spawn_logs(&live, host, engine, names, seq, follow);
                            }
                            Some(Job::StackOp { host, names, action }) => stack_now(host, names, action).await,
                            Some(Job::StopLogs { seq }) => stop_watch(&live, seq, true),
                            Some(Job::StartTerm { host, engine, name, seq, kind }) => {
                                spawn_term(&live, host, engine, name, seq, kind);
                            }
                            Some(Job::TermInput { seq, data }) => send_term(&live, seq, TermEvent::Data(data)),
                            Some(Job::TermResize { seq, cols, rows }) => send_term(&live, seq, TermEvent::Resize { cols, rows }),
                            Some(Job::StopTerm { seq }) => {
                                send_term(&live, seq, TermEvent::Close);
                                stop_watch(&live, seq, false);
                            }
                            Some(Job::Diagnose) => diagnose_now().await,
                            Some(Job::Fix(id)) => fix_now(id).await,
                            Some(Job::CreateNet(spec)) => net_create_now(*spec).await,
                            Some(Job::RemoveNet { host, engine, name }) => net_remove_now(host, engine, name).await,
                            Some(Job::CreateVol(spec)) => vol_create_now(*spec).await,
                            Some(Job::RemoveVol { host, engine, name, pool }) => {
                                vol_remove_now(host, engine, name, pool).await
                            }
                            Some(Job::SnapOp { host, instance, snap, action }) => {
                                snap_now(host, instance, snap, action).await
                            }
                        }
                    }
                    _ = tick.tick() => refresh_now().await,
                    _ = health.tick() => sample_now(&mut sampler).await,
                }
            }
        });
    });
    refresh();
}

fn send(job: Job) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(job);
    }
}

pub fn refresh() {
    send(Job::Refresh);
}

pub fn test_remote(profile: RemoteProfile) {
    send(Job::TestRemote(Box::new(profile)));
}

pub fn operate(host: HostId, engine: Engine, name: impl Into<String>, action: Action) {
    send(Job::Operate { host, engine, name: name.into(), action });
}

pub fn inspect(host: HostId, engine: Engine, name: impl Into<String>) {
    send(Job::Inspect { host, engine, name: name.into() });
}

pub fn stats(host: HostId, engine: Engine, name: impl Into<String>, seq: u64) {
    send(Job::Stats { host, engine, name: name.into(), seq });
}

pub fn create(spec: CreateSpec) {
    send(Job::Create(Box::new(spec)));
}

pub fn pull_image(host: HostId, engine: Engine, reference: impl Into<String>) {
    send(Job::PullImage { host, engine, reference: reference.into() });
}

pub fn inspect_image(host: HostId, engine: Engine, id: impl Into<String>) {
    send(Job::InspectImage { host, engine, id: id.into() });
}

pub fn remove_image(host: HostId, engine: Engine, id: impl Into<String>) {
    send(Job::RemoveImage { host, engine, id: id.into() });
}

pub fn follow_logs(host: HostId, engine: Engine, names: Vec<String>, seq: u64, follow: bool) {
    send(Job::FollowLogs { host, engine, names, seq, follow });
}

pub fn stack_op(host: HostId, names: Vec<String>, action: Action) {
    send(Job::StackOp { host, names, action });
}

pub fn stop_logs(seq: u64) {
    send(Job::StopLogs { seq });
}

pub fn start_term(host: HostId, engine: Engine, name: impl Into<String>, seq: u64, kind: TermKind) {
    send(Job::StartTerm { host, engine, name: name.into(), seq, kind });
}

pub fn term_input(seq: u64, data: Vec<u8>) {
    send(Job::TermInput { seq, data });
}

pub fn term_resize(seq: u64, cols: u16, rows: u16) {
    send(Job::TermResize { seq, cols, rows });
}

pub fn stop_term(seq: u64) {
    send(Job::StopTerm { seq });
}

pub fn shutdown() {
    send(Job::Shutdown);
}

pub fn diagnose() {
    send(Job::Diagnose);
}

pub fn fix(id: CheckId) {
    send(Job::Fix(id));
}

pub fn create_network(spec: NetSpec) {
    send(Job::CreateNet(Box::new(spec)));
}

pub fn remove_network(host: HostId, engine: Engine, name: impl Into<String>) {
    send(Job::RemoveNet { host, engine, name: name.into() });
}

pub fn create_volume(spec: VolumeSpec) {
    send(Job::CreateVol(Box::new(spec)));
}

pub fn remove_volume(host: HostId, engine: Engine, name: impl Into<String>, pool: Option<String>) {
    send(Job::RemoveVol { host, engine, name: name.into(), pool });
}

pub fn snap_op(host: HostId, instance: impl Into<String>, snap: impl Into<String>, action: SnapAction) {
    send(Job::SnapOp { host, instance: instance.into(), snap: snap.into(), action });
}

fn on_main(f: impl FnOnce() + Send + 'static) {
    gtk::glib::idle_add_once(f);
}

async fn refresh_now() {
    let remote = current_remote();
    let snapshot = inventory::collect(&remote).await;
    on_main(move || crate::window::apply_snapshot(snapshot));
}

async fn sample_now(sampler: &mut host::Sampler) {
    let remote = current_remote();
    let health = sampler.sample(&remote).await;
    on_main(move || crate::window::apply_health(health));
}

async fn diagnose_now() {
    let remote = current_remote();
    let report = diagnose::report(&remote).await;
    on_main(move || crate::window::apply_diag(report));
}

async fn fix_now(id: CheckId) {
    let result = tokio::task::spawn_blocking(move || diagnose::run_fix(id)).await.unwrap_or_else(|e| Err(e.to_string()));
    on_main(move || crate::window::apply_fix(result));
}

async fn test_now(profile: RemoteProfile) {
    let results = connections::test_remote(&profile).await;
    on_main(move || crate::window::apply_test(results));
}

async fn operate_now(host: HostId, engine: Engine, name: String, action: Action) {
    let remote = current_remote();
    let result = ops::operate(&remote, host, engine, &name, action).await;
    on_main(move || crate::window::apply_op(result));
}

async fn inspect_now(host: HostId, engine: Engine, name: String) {
    let remote = current_remote();
    let result = ops::inspect(&remote, host, engine, &name).await;
    on_main(move || crate::window::apply_detail(result));
}

async fn stats_now(host: HostId, engine: Engine, name: String, seq: u64) {
    let remote = current_remote();
    let result = ops::stats(&remote, host, engine, &name, seq).await;
    on_main(move || crate::window::apply_stats(result));
}

async fn create_now(spec: CreateSpec) {
    let remote = current_remote();
    let result = ops::create(&remote, &spec).await;
    on_main(move || crate::window::apply_op(result));
}

async fn pull_now(host: HostId, engine: Engine, reference: String) {
    let remote = current_remote();
    let result = ops::pull_image(&remote, host, engine, &reference).await;
    on_main(move || crate::window::apply_op(result));
}

async fn inspect_image_now(host: HostId, engine: Engine, id: String) {
    let remote = current_remote();
    let result = ops::inspect_image(&remote, host, engine, &id).await;
    on_main(move || crate::window::apply_image_detail(result));
}

async fn remove_image_now(host: HostId, engine: Engine, id: String) {
    let remote = current_remote();
    let result = ops::remove_image(&remote, host, engine, &id).await;
    on_main(move || crate::window::apply_op(result));
}

async fn net_create_now(spec: NetSpec) {
    let remote = current_remote();
    let result = ops::create_network(&remote, &spec).await;
    on_main(move || crate::window::apply_op(result));
}

async fn net_remove_now(host: HostId, engine: Engine, name: String) {
    let remote = current_remote();
    let result = ops::remove_network(&remote, host, engine, &name).await;
    on_main(move || crate::window::apply_op(result));
}

async fn vol_create_now(spec: VolumeSpec) {
    let remote = current_remote();
    let result = ops::create_volume(&remote, &spec).await;
    on_main(move || crate::window::apply_op(result));
}

async fn vol_remove_now(host: HostId, engine: Engine, name: String, pool: Option<String>) {
    let remote = current_remote();
    let result = ops::remove_volume(&remote, host, engine, &name, pool.as_deref()).await;
    on_main(move || crate::window::apply_op(result));
}

async fn list_snaps_now(host: HostId, instance: String) {
    let remote = current_remote();
    let result = ops::list_snapshots(&remote, host, &instance).await;
    on_main(move || crate::window::apply_snaps(host, instance, result));
}

async fn snap_now(host: HostId, instance: String, snap: String, action: SnapAction) {
    let remote = current_remote();
    let result = ops::snap_op(&remote, host, &instance, &snap, action).await;
    on_main(move || crate::window::apply_op(result));
    list_snaps_now(host, instance).await;
}

fn spawn_logs(live: &Arc<Mutex<Live>>, host: HostId, engine: Engine, names: Vec<String>, seq: u64, follow: bool) {
    let (stop_tx, stop_rx) = watch::channel(false);
    if let Ok(mut g) = live.lock()
        && let Some(old) = g.logs.insert(seq, stop_tx)
    {
        let _ = old.send(true);
    }
    tokio::spawn(async move {
        let remote = current_remote();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pump = async {
            while let Some(chunk) = rx.recv().await {
                on_main(move || crate::window::apply_log(chunk));
            }
        };
        let work = async {
            let mut tasks = Vec::new();
            for name in names {
                let tx = tx.clone();
                let stop_rx = stop_rx.clone();
                let remote = remote.clone();
                tasks.push(tokio::spawn(async move {
                    ops::follow_logs(&remote, host, engine, &name, seq, follow, tx, stop_rx).await
                }));
            }
            drop(tx);
            let mut last = Ok(());
            for task in tasks {
                match task.await {
                    Ok(Err(err)) => last = Err(err),
                    Err(err) => last = Err(err.to_string()),
                    Ok(Ok(())) => {}
                }
            }
            last
        };
        let (_, result) = tokio::join!(pump, work);
        on_main(move || crate::window::apply_log_done(seq, result));
    });
}

async fn stack_now(host: HostId, names: Vec<String>, action: Action) {
    let remote = current_remote();
    let mut last = Ok(String::new());
    for name in names {
        last = ops::operate(&remote, host, Engine::Docker, &name, action).await;
        if last.is_err() {
            break;
        }
    }
    on_main(move || crate::window::apply_op(last));
}

fn spawn_term(live: &Arc<Mutex<Live>>, host: HostId, engine: Engine, name: String, seq: u64, kind: TermKind) {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    if let Ok(mut g) = live.lock() {
        if let Some(old) = g.terms.insert(seq, ev_tx) {
            let _ = old.send(TermEvent::Close);
        }
        if let Some(old) = g.term_stop.insert(seq, stop_tx) {
            let _ = old.send(true);
        }
    }
    tokio::spawn(async move {
        let remote = current_remote();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let pump = async {
            while let Some(bytes) = rx.recv().await {
                on_main(move || crate::window::apply_term_bytes(seq, bytes));
            }
        };
        let work = ops::session(&remote, host, engine, &name, kind, ev_rx, tx, stop_rx);
        let (_, result) = tokio::join!(pump, work);
        on_main(move || crate::window::apply_term_done(seq, result));
    });
}

fn send_term(live: &Arc<Mutex<Live>>, seq: u64, event: TermEvent) {
    if let Ok(g) = live.lock()
        && let Some(tx) = g.terms.get(&seq)
    {
        let _ = tx.send(event);
    }
}

fn stop_watch(live: &Arc<Mutex<Live>>, seq: u64, logs: bool) {
    if let Ok(mut g) = live.lock() {
        if logs {
            if let Some(tx) = g.logs.remove(&seq) {
                let _ = tx.send(true);
            }
        } else if let Some(tx) = g.term_stop.remove(&seq) {
            let _ = tx.send(true);
            g.terms.remove(&seq);
        }
    }
}

fn current_remote() -> RemoteProfile {
    // Prefs live on the GTK thread. The worker reads the file so it never touches RefCell.
    let path = crate::paths::prefs_file();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<crate::prefs::Prefs>(&text).ok())
        .map(|p| p.remote)
        .unwrap_or_default()
}
