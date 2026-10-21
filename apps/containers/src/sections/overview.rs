use crate::models::{ConnState, EngineStatus, HostFilter, HostHealth, HostId, RunState, Snapshot};
use crate::widgets::{self, OpsTable, Page};
use crate::{settings_dialog, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Overview {
    body: gtk::Box,
}

thread_local! {
    static UI: RefCell<Option<Rc<Overview>>> = const { RefCell::new(None) };
    static HEALTH: RefCell<Vec<HostHealth>> = const { RefCell::new(Vec::new()) };
}

pub fn build(page: &Page) {
    page.body.add_css_class("overview-page");
    UI.with(|u| {
        *u.borrow_mut() = Some(Rc::new(Overview { body: page.body.clone() }));
    });
}

pub fn apply_health(list: Vec<HostHealth>) {
    HEALTH.with(|h| *h.borrow_mut() = list);
    if let Some(snap) = window::snapshot() {
        refresh(&snap, window::host_filter());
    }
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = ui.body.first_child() {
        ui.body.remove(&c);
    }
    let (bar, count, _) = widgets::page_header("Overview");
    ui.body.append(&bar);
    let hosts: Vec<HostId> = match host {
        HostFilter::All => vec![HostId::Local, HostId::Remote],
        HostFilter::Local => vec![HostId::Local],
        HostFilter::Remote => vec![HostId::Remote],
    };
    let mut boards = 0;
    for id in hosts {
        if host == HostFilter::All && id == HostId::Remote && !snapshot.engines.iter().any(|e| e.host == HostId::Remote) {
            continue;
        }
        ui.body.append(&host_panel(snapshot, id));
        boards += 1;
    }
    count.set_text(&if boards == 1 { "1 host".to_string() } else { format!("{boards} hosts") });
    if ui.body.last_child().is_none() {
        ui.body.append(&widgets::empty_state("computer-symbolic", "No host", "Pick Local or add a remote on Connections.", None));
    }
}

fn host_panel(snapshot: &Snapshot, host: HostId) -> gtk::Box {
    let root = widgets::vbox(12);
    root.add_css_class("host-panel");
    let title = widgets::label(&window::host_name(host), "host-panel-title");
    title.set_xalign(0.0);
    root.append(&title);

    let chips = widgets::vbox(4);
    chips.add_css_class("engine-chips");
    let engines: Vec<_> = snapshot.engines.iter().filter(|e| e.host == host).collect();
    if engines.is_empty() {
        chips.append(&chip("No engines", "This host has no Docker or Incus profile.", "", false));
    }
    for st in engines {
        chips.append(&engine_chip(st));
    }
    root.append(&chips);

    let health = HEALTH.with(|h| h.borrow().iter().find(|x| x.host == host).cloned());
    let tiles = gtk::FlowBox::new();
    tiles.add_css_class("metric-grid");
    tiles.set_selection_mode(gtk::SelectionMode::None);
    tiles.set_max_children_per_line(4);
    tiles.set_min_children_per_line(2);
    tiles.set_row_spacing(10);
    tiles.set_column_spacing(10);
    tiles.set_homogeneous(true);
    let cpu = cpu_text(&health);
    tiles.append(&metric_card(
        "CPU",
        &cpu,
        health.as_ref().and_then(|h| h.cpu).map(|v| v / 100.0),
        health.as_ref().map(|h| h.cpu_hist.as_slice()).unwrap_or(&[]),
    ));
    let mem = mem_text(&health);
    tiles.append(&metric_card(
        "Memory",
        &mem,
        frac(health.as_ref().and_then(|h| h.mem_used), health.as_ref().and_then(|h| h.mem_total)),
        health.as_ref().map(|h| h.mem_hist.as_slice()).unwrap_or(&[]),
    ));
    let disk = disk_text(&health);
    tiles.append(&metric_card(
        "Engine disk",
        &disk,
        frac(health.as_ref().and_then(|h| h.disk_used), health.as_ref().and_then(|h| h.disk_total)),
        health.as_ref().map(|h| h.disk_hist.as_slice()).unwrap_or(&[]),
    ));
    let running = snapshot.running(filter_of(host), None);
    let total = snapshot.filtered(filter_of(host), None).count();
    let guests_frac = if total == 0 { None } else { Some(running as f64 / total as f64) };
    let guests = format!("{running} / {total}");
    tiles.append(&metric_card("Guests", &guests, guests_frac, &[]));
    root.append(&tiles);

    let table = OpsTable::new(
        &[
            widgets::col("name", "Name", true, false, 0),
            widgets::col("engine", "Engine", false, false, 80),
            widgets::col("type", "Type", false, false, 96),
            widgets::col("status", "Status", false, false, 88),
        ],
        "application-x-executable-symbolic",
        "Nothing is running",
        "Start a container or VM to see it here.",
    );
    let running_head = widgets::label("Running now", "running-head");
    running_head.set_xalign(0.0);
    root.append(&running_head);
    let mut shown = 0;
    for w in snapshot.filtered(filter_of(host), None).filter(|w| w.state == RunState::Running) {
        let row = table.push(vec![
            widgets::td(&w.name),
            widgets::td_dim(w.engine.label()),
            widgets::td_dim(w.kind.label()),
            widgets::status_pill(w.state.label(), "ok").upcast(),
        ]);
        let guest = w.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| crate::sections::workloads::reveal(&guest));
        row.add_controller(click);
        shown += 1;
    }
    table.set_count(shown);
    root.append(&table.root);
    let stopped = snapshot.stopped(filter_of(host), None);
    if stopped > 0 {
        let foot = widgets::label(&format!("{stopped} stopped"), "dim");
        foot.set_xalign(0.0);
        foot.add_css_class("running-foot");
        root.append(&foot);
    }
    root
}

fn filter_of(host: HostId) -> HostFilter {
    match host {
        HostId::Local => HostFilter::Local,
        HostId::Remote => HostFilter::Remote,
    }
}

fn cpu_text(h: &Option<HostHealth>) -> String {
    h.as_ref().and_then(|h| h.cpu).map(|v| format!("{v:.0}%")).unwrap_or_else(|| "—".into())
}

fn mem_text(h: &Option<HostHealth>) -> String {
    match h {
        Some(h) => crate::models::format_memory(h.mem_used, h.mem_total),
        None => "—".into(),
    }
}

fn disk_text(h: &Option<HostHealth>) -> String {
    match h {
        Some(h) => crate::models::format_memory(h.disk_used, h.disk_total),
        None => "—".into(),
    }
}

fn frac(used: Option<u64>, total: Option<u64>) -> Option<f64> {
    match (used, total) {
        (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64),
        _ => None,
    }
}

fn metric_card(title: &str, value: &str, fraction: Option<f64>, hist: &[f64]) -> gtk::Box {
    let card = widgets::vbox(6);
    card.add_css_class("metric-card");
    let t = widgets::label(title, "metric-label");
    t.set_xalign(0.0);
    card.append(&t);
    let v = widgets::label(value, "metric-value");
    v.set_xalign(0.0);
    card.append(&v);
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(fraction.unwrap_or(0.0));
    if fraction.is_none() {
        bar.add_css_class("empty");
    }
    card.append(&bar);
    card.append(&widgets::sparkline(hist));
    card
}

fn engine_chip(st: &EngineStatus) -> gtk::Box {
    let (status, kind, detail, fix) = match &st.state {
        ConnState::Connected { detail } => ("Connected", "ok", detail.as_str(), false),
        ConnState::Unavailable { reason } => ("Down", "danger", reason.as_str(), true),
        ConnState::Permission { reason } => ("Permission", "danger", reason.as_str(), true),
        ConnState::Unconfigured => ("Off", "", "Not configured.", false),
        ConnState::Untrusted { .. } => ("Untrusted", "danger", "Unknown identity.", true),
        ConnState::Insecure { reason } => ("Blocked", "danger", reason.as_str(), true),
    };
    chip(&format!("{} {status}", st.engine.label()), detail, kind, fix)
}

fn chip(title: &str, detail: &str, kind: &str, fix: bool) -> gtk::Box {
    let row = widgets::hbox(8);
    row.add_css_class("engine-chip");
    let tag = widgets::tag(title, kind);
    row.append(&tag);
    let d = widgets::label(detail, "dim");
    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
    d.set_hexpand(true);
    d.set_xalign(0.0);
    row.append(&d);
    if fix {
        let b = gtk::Button::with_label("Fix");
        b.add_css_class("suggested-action");
        b.connect_clicked(|_| {
            settings_dialog::open_diagnostics();
        });
        row.append(&b);
    }
    row
}
