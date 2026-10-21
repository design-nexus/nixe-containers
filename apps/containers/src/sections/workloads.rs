use crate::models::{
    Action, CreateSpec, Engine, HostFilter, HostId, InstanceSnap, Kind, ResourceSample, RunState, SnapAction, Snapshot,
    Workload, WorkloadDetail,
};
use crate::widgets::{self, Page};
use crate::{prefs, window, worker};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct List {
    kind: Option<Kind>,
    rows: gtk::Box,
    empty: gtk::Box,
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

pub fn build_containers(page: &Page) {
    build(page, None, &CONTAINERS);
}

pub fn build_vms(page: &Page) {
    build(page, Some(Kind::VirtualMachine), &VMS);
}

fn build(page: &Page, kind: Option<Kind>, slot: &'static std::thread::LocalKey<RefCell<Option<Rc<List>>>>) {
    page.fill();
    let bar = widgets::hbox(8);
    bar.add_css_class("page-toolbar");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let create = gtk::Button::with_label("Create");
    create.add_css_class("suggested-action");
    create.connect_clicked(move |_| open_create(kind));
    bar.append(&create);
    page.body.append(&bar);
    let header = row(["Name", "Host", "Engine", "Type", "Status"], true);
    header.add_css_class("workload-header");
    page.body.append(&header);
    let rows = widgets::vbox(0);
    rows.set_vexpand(true);
    let empty = widgets::empty_state(
        "application-x-executable-symbolic",
        if kind == Some(Kind::VirtualMachine) { "No virtual machines" } else { "No containers" },
        "Nothing matches this host and filter.",
        None,
    );
    empty.set_visible(false);
    page.body.append(&rows);
    page.body.append(&empty);
    slot.with(|s| {
        *s.borrow_mut() = Some(Rc::new(List { kind, rows, empty }));
    });
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    refresh_one(&CONTAINERS, snapshot, host, query);
    refresh_one(&VMS, snapshot, host, query);
}

fn refresh_one(
    slot: &'static std::thread::LocalKey<RefCell<Option<Rc<List>>>>,
    snapshot: &Snapshot,
    host: HostFilter,
    query: &str,
) {
    let Some(list) = slot.with(|s| s.borrow().clone()) else { return };
    while let Some(c) = list.rows.first_child() {
        list.rows.remove(&c);
    }
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    for w in snapshot.filtered(host, list.kind) {
        if !q.is_empty() && !matches_query(w, &q) {
            continue;
        }
        list.rows.append(&workload_row(w));
        shown += 1;
    }
    list.rows.set_visible(shown > 0);
    list.empty.set_visible(shown == 0);
}

fn matches_query(w: &Workload, q: &str) -> bool {
    w.name.to_ascii_lowercase().contains(q)
        || w.engine.label().to_ascii_lowercase().contains(q)
        || w.host.label().to_ascii_lowercase().contains(q)
        || w.status.to_ascii_lowercase().contains(q)
}

fn workload_row(w: &Workload) -> gtk::Box {
    let status_kind = match w.state {
        RunState::Running => "ok",
        RunState::Stopped => "",
        RunState::Paused => "",
        RunState::Other => "danger",
    };
    let root = row([&w.name, &window::host_name(w.host), w.engine.label(), w.kind.label(), w.state.label()], false);
    root.add_css_class("workload-row");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    match w.state {
        RunState::Running => dot.add_css_class("ok"),
        RunState::Other => dot.add_css_class("danger"),
        _ => {}
    }
    root.prepend(&dot);
    if let Some(status) = root.last_child().and_downcast::<gtk::Label>() {
        status.add_css_class("tag");
        if !status_kind.is_empty() {
            status.add_css_class(status_kind);
        }
    }
    root.append(&action_buttons(w));
    root.set_tooltip_text(Some(&format!("{} · {}", w.status, w.image.as_deref().unwrap_or(""))));
    let target = (w.host, w.engine, w.target_id().to_string());
    let click = gtk::GestureClick::new();
    click.set_button(1);
    click.connect_released(move |g, _, x, y| {
        if let Some(picked) = g.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT))
            && (picked.is::<gtk::Button>() || picked.ancestor(gtk::Button::static_type()).is_some())
        {
            return;
        }
        worker::inspect(target.0, target.1, target.2.clone());
    });
    root.add_controller(click);
    root
}

fn action_buttons(w: &Workload) -> gtk::Box {
    let box_ = widgets::hbox(4);
    box_.add_css_class("col-actions");
    box_.set_halign(gtk::Align::End);
    let (host, engine, name, id, kind) = (w.host, w.engine, w.name.clone(), w.target_id().to_string(), w.kind);
    if w.state == RunState::Running {
        box_.append(&op_button("media-playback-stop-symbolic", "Stop", host, engine, id.clone(), Action::Stop));
        box_.append(&op_button("view-refresh-symbolic", "Restart", host, engine, id.clone(), Action::Restart));
    } else {
        box_.append(&op_button("media-playback-start-symbolic", "Start", host, engine, id.clone(), Action::Start));
    }
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.add_css_class("icon-button");
    del.set_tooltip_text(Some("Delete"));
    let host_label = window::host_name(host);
    del.connect_clicked(move |_| {
        confirm_delete(kind, &name, &id, host, engine, &host_label);
    });
    box_.append(&del);
    box_
}

fn op_button(icon: &str, tip: &str, host: HostId, engine: Engine, name: String, action: Action) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.add_css_class("icon-button");
    b.set_tooltip_text(Some(tip));
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

pub fn show_detail(detail: WorkloadDetail) {
    let seq = DETAIL_GEN.with(|g| {
        let n = g.get() + 1;
        g.set(n);
        n
    });
    let (dialog, card) = widgets::dialog(&detail.name, 520);
    let status = widgets::tag(
        detail.state.label(),
        match detail.state {
            RunState::Running => "ok",
            RunState::Other => "danger",
            _ => "",
        },
    );
    card.append(&widgets::row("Status", &detail.status, Some(status.upcast_ref())));
    card.append(&kv("Host", &window::host_name(detail.host)));
    card.append(&kv("Engine", detail.engine.label()));
    card.append(&kv("Type", detail.kind.label()));
    if let Some(image) = &detail.image {
        card.append(&kv("Image", image));
    }
    if let Some(created) = &detail.created {
        card.append(&kv("Created", created));
    }
    if let Some(cmd) = &detail.command {
        card.append(&kv("Command", cmd));
    }
    if let Some(ip) = &detail.ip {
        card.append(&kv("Address", ip));
    }
    let cpu = widgets::label(&crate::models::format_cpu(detail.cpu), "");
    let memory = widgets::label(&crate::models::format_memory(detail.memory_used, detail.memory_limit), "");
    card.append(&widgets::row("CPU", "Live sample from the engine.", Some(cpu.upcast_ref())));
    card.append(&widgets::row("Memory", "", Some(memory.upcast_ref())));
    for (k, v) in &detail.extra {
        card.append(&kv(k, v));
    }
    let snaps = widgets::vbox(4);
    if detail.engine == Engine::Incus {
        let title = widgets::label("SNAPSHOTS", "group-title");
        title.set_xalign(0.0);
        card.append(&title);
        fill_snaps(&snaps, &detail.snapshots);
        card.append(&snaps);
    }
    let actions = widgets::hbox(8);
    actions.set_halign(gtk::Align::End);
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
    crate::sections::session::add_detail_actions(&actions, host, engine, &name, kind, state);
    if engine == Engine::Incus {
        let snap = gtk::Button::with_label("Snapshot");
        let n = name.clone();
        snap.connect_clicked(move |_| open_create_snap(host, n.clone()));
        actions.append(&snap);
    }
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    let host_label = window::host_name(host);
    let n = name.clone();
    delete.connect_clicked(move |_| confirm_delete(kind, &n, &n, host, engine, &host_label));
    actions.append(&delete);
    card.append(&actions);
    let ui = Rc::new(DetailUi { cpu, memory, snaps, host, engine, name: name.clone(), seq });
    DETAIL.with(|d| *d.borrow_mut() = Some(ui.clone()));
    dialog.connect_close_request(move |_| {
        DETAIL_GEN.with(|g| g.set(g.get().wrapping_add(1)));
        DETAIL.with(|d| *d.borrow_mut() = None);
        glib::Propagation::Proceed
    });
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
    dialog.present();
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
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.add_css_class("icon-button");
        del.set_tooltip_text(Some("Delete snapshot"));
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

fn kv(title: &str, value: &str) -> gtk::Box {
    widgets::row(title, "", Some(widgets::label(value, "").upcast_ref()))
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

fn row(cols: [&str; 5], header: bool) -> gtk::Box {
    let b = widgets::hbox(12);
    let classes = ["col-name", "col-host", "col-engine", "col-type", "col-status"];
    for (i, text) in cols.iter().enumerate() {
        let l = widgets::label(text, if header { "dim" } else { "" });
        l.add_css_class(classes[i]);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if i == 0 {
            l.set_hexpand(true);
        }
        b.append(&l);
    }
    b
}
