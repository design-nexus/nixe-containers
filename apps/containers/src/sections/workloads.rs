use crate::models::{
    Action, CreateSpec, Engine, HostFilter, HostId, InstanceSnap, Kind, ResourceSample, RunState, SnapAction, Snapshot,
    Workload, WorkloadDetail,
};
use crate::widgets::{self, Col, OpsTable, Page};
use crate::{prefs, window, worker};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct List {
    kind: Option<Kind>,
    table: OpsTable,
    count: gtk::Label,
    inspector: gtk::Box,
    selected: RefCell<Option<String>>,
    suppress: Cell<bool>,
}

struct DetailUi {
    cpu: gtk::Label,
    memory: gtk::Label,
    snaps: gtk::Box,
    host: HostId,
    engine: Engine,
    name: String,
    seq: u64,
}

thread_local! {
    static CONTAINERS: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
    static VMS: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
    static DETAIL: RefCell<Option<Rc<DetailUi>>> = const { RefCell::new(None) };
    static DETAIL_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn cols() -> [Col; 6] {
    [
        widgets::col("name", "Name", true, false, 0),
        widgets::col("host", "Host", false, false, 88),
        widgets::col("engine", "Engine", false, false, 80),
        widgets::col("type", "Type", false, false, 96),
        widgets::col("status", "Status", false, false, 88),
        widgets::col("actions", "", false, true, 112),
    ]
}

pub fn build_containers(page: &Page) {
    build(page, Some(Kind::Container), &CONTAINERS);
}

pub fn build_vms(page: &Page) {
    build(page, Some(Kind::VirtualMachine), &VMS);
}

fn build(page: &Page, kind: Option<Kind>, slot: &'static std::thread::LocalKey<RefCell<Option<Rc<List>>>>) {
    page.fill();
    let (bar, count, actions) = widgets::page_header();
    let create = widgets::primary_button(
        "list-add-symbolic",
        if kind == Some(Kind::VirtualMachine) { "New virtual machine" } else { "New container" },
    );
    create.connect_clicked(move |_| open_create(kind));
    actions.append(&create);
    page.body.append(&bar);

    let table = OpsTable::new(
        &cols(),
        if kind == Some(Kind::VirtualMachine) { "computer-symbolic" } else { "application-x-executable-symbolic" },
        if kind == Some(Kind::VirtualMachine) { "No virtual machines" } else { "No containers" },
        "Nothing matches this host and filter.",
    );
    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inspector.add_css_class("ops-inspector");
    inspector.set_hexpand(false);
    inspector.set_size_request(320, -1);
    inspector_idle(&inspector);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_wide_handle(true);
    paned.set_start_child(Some(&table.root));
    paned.set_end_child(Some(&inspector));
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(false);
    paned.set_shrink_end_child(false);
    paned.set_vexpand(true);
    page.body.append(&paned);

    let list = Rc::new(List { kind, table, count, inspector, selected: RefCell::new(None), suppress: Cell::new(false) });
    let list_sel = list.clone();
    list.table.listbox().connect_row_selected(move |_, row| {
        if list_sel.suppress.get() {
            return;
        }
        let Some(row) = row else {
            list_sel.selected.replace(None);
            inspector_idle(&list_sel.inspector);
            DETAIL.with(|d| *d.borrow_mut() = None);
            return;
        };
        let key = row.widget_name().to_string();
        if list_sel.selected.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        list_sel.selected.replace(Some(key.clone()));
        if let Some((host, engine, id)) = parse_key(&key) {
            worker::inspect(host, engine, id);
        }
    });
    slot.with(|s| {
        *s.borrow_mut() = Some(list);
    });
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    refresh_one(&CONTAINERS, snapshot, host, query, "container");
    refresh_one(&VMS, snapshot, host, query, "virtual machine");
}

fn refresh_one(
    slot: &'static std::thread::LocalKey<RefCell<Option<Rc<List>>>>,
    snapshot: &Snapshot,
    host: HostFilter,
    query: &str,
    noun: &str,
) {
    let Some(list) = slot.with(|s| s.borrow().clone()) else { return };
    list.suppress.set(true);
    list.table.clear();
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    let keep = list.selected.borrow().clone();
    let mut found = None;
    for w in snapshot.filtered(host, list.kind) {
        if !q.is_empty() && !matches_query(w, &q) {
            continue;
        }
        let key = row_key(w);
        let row = list.table.push(workload_cells(w));
        row.set_widget_name(&key);
        row.set_tooltip_text(Some(&format!("{} · {}", w.status, w.image.as_deref().unwrap_or(""))));
        if keep.as_deref() == Some(key.as_str()) {
            found = Some(row);
        }
        shown += 1;
    }
    list.table.set_count(shown);
    widgets::set_count(&list.count, shown, noun);
    if let Some(row) = found {
        list.table.listbox().select_row(Some(&row));
    } else if keep.is_some() {
        list.selected.replace(None);
        inspector_idle(&list.inspector);
        DETAIL.with(|d| *d.borrow_mut() = None);
    }
    list.suppress.set(false);
}

fn matches_query(w: &Workload, q: &str) -> bool {
    w.name.to_ascii_lowercase().contains(q)
        || w.engine.label().to_ascii_lowercase().contains(q)
        || w.host.label().to_ascii_lowercase().contains(q)
        || w.status.to_ascii_lowercase().contains(q)
}

fn row_key(w: &Workload) -> String {
    format!("{}|{}|{}", w.host.label(), w.engine.label(), w.target_id())
}

fn parse_key(key: &str) -> Option<(HostId, Engine, String)> {
    let mut parts = key.splitn(3, '|');
    let host = match parts.next()? {
        "Local" => HostId::Local,
        "Remote" => HostId::Remote,
        _ => return None,
    };
    let engine = match parts.next()? {
        "Docker" => Engine::Docker,
        "Incus" => Engine::Incus,
        _ => return None,
    };
    let id = parts.next()?.to_string();
    if id.is_empty() {
        None
    } else {
        Some((host, engine, id))
    }
}

fn workload_cells(w: &Workload) -> Vec<gtk::Widget> {
    let kind = match w.state {
        RunState::Running => "ok",
        RunState::Other => "danger",
        _ => "",
    };
    let actions = action_buttons(w);
    vec![
        widgets::td(&w.name),
        widgets::td_dim(&window::host_name(w.host)),
        widgets::td_dim(w.engine.label()),
        widgets::td_dim(w.kind.label()),
        widgets::status_pill(w.state.label(), kind).upcast(),
        actions.upcast(),
    ]
}

fn action_buttons(w: &Workload) -> gtk::Box {
    let box_ = widgets::actions_cell();
    let (host, engine, name, id, kind) = (w.host, w.engine, w.name.clone(), w.target_id().to_string(), w.kind);
    if w.state == RunState::Running {
        box_.append(&op_button("media-playback-stop-symbolic", "Stop", host, engine, id.clone(), Action::Stop));
        box_.append(&op_button("view-refresh-symbolic", "Restart", host, engine, id.clone(), Action::Restart));
    } else {
        box_.append(&op_button("media-playback-start-symbolic", "Start", host, engine, id.clone(), Action::Start));
    }
    let del = widgets::icon_button("user-trash-symbolic", "Delete");
    let host_label = window::host_name(host);
    del.connect_clicked(move |_| {
        confirm_delete(kind, &name, &id, host, engine, &host_label);
    });
    box_.append(&del);
    box_
}

fn op_button(icon: &str, tip: &str, host: HostId, engine: Engine, name: String, action: Action) -> gtk::Button {
    let b = widgets::icon_button(icon, tip);
    b.connect_clicked(move |_| {
        window::toast(&format!("{} {name} on {}…", action.gerund(), window::host_name(host)));
        worker::operate(host, engine, name.clone(), action);
    });
    b
}

fn confirm_delete(kind: Kind, name: &str, id: &str, host: HostId, engine: Engine, host_label: &str) {
    let title = format!("Delete {}?", kind.label().to_ascii_lowercase());
    let body = format!("Delete {} {name} on {host_label}? This cannot be undone.", kind.label().to_ascii_lowercase());
    let name = name.to_string();
    let id = id.to_string();
    widgets::confirm(&title, &body, "Delete", move || {
        window::toast(&format!("Deleting {name} on {}…", window::host_name(host)));
        worker::operate(host, engine, id.clone(), Action::Delete);
    });
}

pub fn reveal(w: &Workload) {
    if w.kind == Kind::VirtualMachine {
        window::navigate("vms");
    } else {
        window::navigate("containers");
    }
    worker::inspect(w.host, w.engine, w.target_id());
}

fn inspector_idle(box_: &gtk::Box) {
    while let Some(c) = box_.first_child() {
        box_.remove(&c);
    }
    box_.append(&widgets::empty_state(
        "view-paged-symbolic",
        "Select a guest",
        "Status, resources, logs, and terminal stay on this pane.",
        None,
    ));
}

pub fn show_detail(detail: WorkloadDetail) {
    let slot = if detail.kind == Kind::VirtualMachine { &VMS } else { &CONTAINERS };
    let Some(list) = slot.with(|s| s.borrow().clone()) else { return };
    let seq = DETAIL_GEN.with(|g| {
        let n = g.get() + 1;
        g.set(n);
        n
    });
    while let Some(c) = list.inspector.first_child() {
        list.inspector.remove(&c);
    }
    let body = widgets::vbox(10);
    body.set_hexpand(true);
    let head = widgets::hbox(8);
    let title = widgets::label(&detail.name, "ops-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&title);
    head.append(&widgets::status_pill(
        detail.state.label(),
        match detail.state {
            RunState::Running => "ok",
            RunState::Other => "danger",
            _ => "",
        },
    ));
    body.append(&head);
    body.append(&widgets::kv_line("Status", &detail.status));
    body.append(&widgets::kv_line("Host", &window::host_name(detail.host)));
    body.append(&widgets::kv_line("Engine", detail.engine.label()));
    body.append(&widgets::kv_line("Type", detail.kind.label()));
    if let Some(image) = &detail.image {
        body.append(&widgets::kv_line("Image", image));
    }
    if let Some(created) = &detail.created {
        body.append(&widgets::kv_line("Created", created));
    }
    if let Some(cmd) = &detail.command {
        body.append(&widgets::kv_line("Command", cmd));
    }
    if let Some(ip) = &detail.ip {
        body.append(&widgets::kv_line("Address", ip));
    }
    let cpu = widgets::label(&crate::models::format_cpu(detail.cpu), "");
    cpu.set_xalign(0.0);
    let memory = widgets::label(&crate::models::format_memory(detail.memory_used, detail.memory_limit), "");
    memory.set_xalign(0.0);
    body.append(&widgets::row("CPU", "Live sample from the engine.", Some(cpu.upcast_ref())));
    body.append(&widgets::row("Memory", "", Some(memory.upcast_ref())));
    for (k, v) in &detail.extra {
        body.append(&widgets::kv_line(k, v));
    }
    let snaps = widgets::vbox(4);
    if detail.engine == Engine::Incus {
        let title = widgets::label("SNAPSHOTS", "group-title");
        title.set_xalign(0.0);
        body.append(&title);
        fill_snaps(&snaps, &detail.snapshots);
        body.append(&snaps);
    }
    let actions = gtk::FlowBox::new();
    actions.set_selection_mode(gtk::SelectionMode::None);
    actions.set_max_children_per_line(3);
    actions.set_min_children_per_line(2);
    actions.set_row_spacing(6);
    actions.set_column_spacing(6);
    let (host, engine, name, kind, state) = (detail.host, detail.engine, detail.name.clone(), detail.kind, detail.state);
    if state == RunState::Running {
        let stop = gtk::Button::with_label("Stop");
        let n = name.clone();
        stop.connect_clicked(move |_| worker::operate(host, engine, n.clone(), Action::Stop));
        let restart = gtk::Button::with_label("Restart");
        let n = name.clone();
        restart.connect_clicked(move |_| worker::operate(host, engine, n.clone(), Action::Restart));
        actions.append(&stop);
        actions.append(&restart);
    } else {
        let start = gtk::Button::with_label("Start");
        start.add_css_class("suggested-action");
        let n = name.clone();
        start.connect_clicked(move |_| worker::operate(host, engine, n.clone(), Action::Start));
        actions.append(&start);
    }
    let action_row = widgets::hbox(8);
    action_row.set_homogeneous(false);
    crate::sections::session::add_detail_actions(&action_row, host, engine, &name, kind, state);
    if engine == Engine::Incus {
        let snap = gtk::Button::with_label("Snapshot");
        let n = name.clone();
        snap.connect_clicked(move |_| open_create_snap(host, n.clone()));
        action_row.append(&snap);
    }
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    let host_label = window::host_name(host);
    let n = name.clone();
    delete.connect_clicked(move |_| confirm_delete(kind, &n, &n, host, engine, &host_label));
    action_row.append(&delete);
    body.append(&actions);
    body.append(&action_row);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .child(&body)
        .build();
    list.inspector.append(&scroll);
    let ui = Rc::new(DetailUi { cpu, memory, snaps, host, engine, name: name.clone(), seq });
    DETAIL.with(|d| *d.borrow_mut() = Some(ui.clone()));
    if state == RunState::Running {
        worker::stats(host, engine, name, seq);
        glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
            let live = DETAIL.with(|d| d.borrow().as_ref().is_some_and(|u| u.seq == seq));
            if live {
                worker::stats(host, engine, ui.name.clone(), seq);
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

pub fn apply_snaps(host: HostId, instance: &str, snaps: &[InstanceSnap]) {
    let Some(ui) = DETAIL.with(|d| d.borrow().clone()) else { return };
    if ui.host != host || ui.engine != Engine::Incus || ui.name != instance {
        return;
    }
    fill_snaps(&ui.snaps, snaps);
}

fn fill_snaps(box_: &gtk::Box, snaps: &[InstanceSnap]) {
    while let Some(c) = box_.first_child() {
        box_.remove(&c);
    }
    if snaps.is_empty() {
        box_.append(&widgets::label("No snapshots", "dim"));
        return;
    }
    for snap in snaps {
        let row = widgets::hbox(8);
        let name = widgets::label(&snap.name, "");
        name.set_hexpand(true);
        name.set_xalign(0.0);
        row.append(&name);
        if let Some(created) = &snap.created {
            row.append(&widgets::label(created, "dim"));
        }
        if snap.stateful {
            row.append(&widgets::label("stateful", "dim"));
        }
        let restore = gtk::Button::with_label("Restore");
        let host = snap.host;
        let inst = snap.instance.clone();
        let snap_name = snap.name.clone();
        let host_label = window::host_name(host);
        restore.connect_clicked(move |_| {
            let body = format!("Restore {inst} on {host_label} to snapshot {snap_name}? Current state is replaced.");
            let (inst, snap_name) = (inst.clone(), snap_name.clone());
            widgets::confirm("Restore snapshot?", &body, "Restore", move || {
                window::toast(&format!("Restoring {inst} to {snap_name}…"));
                worker::snap_op(host, inst.clone(), snap_name.clone(), SnapAction::Restore);
            });
        });
        let del = widgets::icon_button("user-trash-symbolic", "Delete snapshot");
        let host = snap.host;
        let inst = snap.instance.clone();
        let snap_name = snap.name.clone();
        let host_label = window::host_name(host);
        del.connect_clicked(move |_| {
            let body = format!("Delete snapshot {snap_name} of {inst} on {host_label}? This cannot be undone.");
            let (inst, snap_name) = (inst.clone(), snap_name.clone());
            widgets::confirm("Delete snapshot?", &body, "Delete", move || {
                window::toast(&format!("Deleting snapshot {snap_name}…"));
                worker::snap_op(host, inst.clone(), snap_name.clone(), SnapAction::Delete);
            });
        });
        row.append(&restore);
        row.append(&del);
        box_.append(&row);
    }
}

fn open_create_snap(host: HostId, instance: String) {
    let (dialog, card) = widgets::dialog(&format!("Snapshot {instance}"), 440);
    let name = gtk::Entry::new();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    name.set_text(&format!("snap-{secs}"));
    name.set_hexpand(true);
    card.append(&widgets::row("Name", "", Some(name.upcast_ref())));
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label("Create");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let d = dialog.clone();
    ok.connect_clicked(move |_| {
        let snap = name.text().trim().to_string();
        if snap.is_empty() {
            window::toast("A snapshot name is required.");
            return;
        }
        window::toast(&format!("Creating snapshot {snap}…"));
        worker::snap_op(host, instance.clone(), snap, SnapAction::Create);
        d.close();
    });
    dialog.present();
}

pub fn apply_stats(sample: ResourceSample) {
    let Some(ui) = DETAIL.with(|d| d.borrow().clone()) else { return };
    if ui.seq != sample.seq || ui.host != sample.host || ui.engine != sample.engine || ui.name != sample.name {
        return;
    }
    ui.cpu.set_text(&crate::models::format_cpu(sample.cpu));
    ui.memory.set_text(&crate::models::format_memory(sample.memory_used, sample.memory_limit));
}

fn open_create(kind: Option<Kind>) {
    let (dialog, card) =
        widgets::dialog(if kind == Some(Kind::VirtualMachine) { "Create virtual machine" } else { "Create container" }, 480);
    let host = Rc::new(RefCell::new(HostId::Local));
    let engine = Rc::new(RefCell::new(if kind == Some(Kind::VirtualMachine) { Engine::Incus } else { Engine::Docker }));
    let kind_cell = Rc::new(RefCell::new(kind.unwrap_or(Kind::Container)));
    card.append(&widgets::segmented_row(
        "Host",
        "Where the workload will run.",
        vec![("local".into(), "Local".into()), ("remote".into(), window::host_name(HostId::Remote))],
        "local",
        {
            let host = host.clone();
            move |id| *host.borrow_mut() = if id == "remote" { HostId::Remote } else { HostId::Local }
        },
    ));
    card.append(&widgets::segmented_row(
        "Engine",
        "Docker containers, or Incus containers and VMs.",
        vec![("docker".into(), "Docker".into()), ("incus".into(), "Incus".into())],
        if kind == Some(Kind::VirtualMachine) { "incus" } else { "docker" },
        {
            let engine = engine.clone();
            let kind_cell = kind_cell.clone();
            move |id| {
                *engine.borrow_mut() = if id == "incus" { Engine::Incus } else { Engine::Docker };
                if *engine.borrow() == Engine::Docker {
                    *kind_cell.borrow_mut() = Kind::Container;
                }
            }
        },
    ));
    if kind != Some(Kind::VirtualMachine) {
        card.append(&widgets::segmented_row(
            "Type",
            "Virtual machines need Incus.",
            vec![("container".into(), "Container".into()), ("vm".into(), "Virtual machine".into())],
            "container",
            {
                let engine = engine.clone();
                let kind_cell = kind_cell.clone();
                move |id| {
                    if id == "vm" {
                        *kind_cell.borrow_mut() = Kind::VirtualMachine;
                        *engine.borrow_mut() = Engine::Incus;
                    } else {
                        *kind_cell.borrow_mut() = Kind::Container;
                    }
                }
            },
        ));
    }
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name"));
    name.set_hexpand(true);
    card.append(&widgets::row("Name", "", Some(name.upcast_ref())));
    let image = gtk::Entry::new();
    image.set_placeholder_text(Some(if kind == Some(Kind::VirtualMachine) { "ubuntu/24.04" } else { "nginx:latest" }));
    image.set_hexpand(true);
    card.append(&widgets::row("Image", "Docker tag, or an Incus alias already on the host.", Some(image.upcast_ref())));
    let start = gtk::Switch::new();
    start.set_active(true);
    card.append(&widgets::row("Start after create", "", Some(start.upcast_ref())));
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label("Create");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let d = dialog.clone();
    ok.connect_clicked(move |_| {
        let n = name.text().trim().to_string();
        let img = image.text().trim().to_string();
        if n.is_empty() || img.is_empty() {
            window::toast("Name and image are required.");
            return;
        }
        let spec = CreateSpec {
            host: *host.borrow(),
            engine: *engine.borrow(),
            kind: *kind_cell.borrow(),
            name: n,
            image: img,
            start: start.is_active(),
        };
        if spec.engine == Engine::Docker && spec.kind == Kind::VirtualMachine {
            window::toast("Docker does not run virtual machines. Choose Incus.");
            return;
        }
        if spec.host == HostId::Remote && !prefs::get().remote.configured() {
            window::toast("Configure a remote host first.");
            return;
        }
        window::toast(&format!("Creating {} on {}…", spec.name, window::host_name(spec.host)));
        worker::create(spec);
        d.close();
    });
    dialog.present();
}
