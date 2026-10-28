use crate::models::{Engine, HostFilter, HostId, Snapshot, VolumeRow, VolumeSpec};
use crate::widgets::{self, Col, OpsTable, Page};
use crate::{prefs, window, worker};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct List {
    table: OpsTable,
    count: gtk::Label,
}

thread_local! {
    static UI: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
}

fn cols() -> [Col; 6] {
    [
        widgets::col("name", "Name", true, false, 0),
        widgets::col("host", "Host", false, false, 88),
        widgets::col("engine", "Engine", false, false, 80),
        widgets::col("pool", "Pool / driver", false, false, 140),
        widgets::col("size", "Size", false, true, 88),
        widgets::col("actions", "", false, true, 48),
    ]
}

pub fn build(page: &Page) {
    page.fill();
    let (bar, count, actions) = widgets::page_header();
    let create = widgets::primary_button("list-add-symbolic", "New volume");
    create.connect_clicked(|_| open_create());
    actions.append(&create);
    page.body.append(&bar);
    let table = OpsTable::new(
        &cols(),
        "drive-harddisk-symbolic",
        "No volumes",
        "Nothing matches this host filter. Create a volume to add one.",
    );
    page.body.append(&table.root);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { table, count })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(list) = UI.with(|u| u.borrow().clone()) else { return };
    list.table.clear();
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    for vol in snapshot.volumes(host) {
        if !q.is_empty()
            && !vol.name.to_ascii_lowercase().contains(&q)
            && !vol.engine.label().to_ascii_lowercase().contains(&q)
            && !vol.kind.to_ascii_lowercase().contains(&q)
            && !vol.pool.as_deref().unwrap_or("").to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        let row = list.table.push(vol_cells(vol));
        row.set_tooltip_text(Some(&format!("{} attached", vol.used_by)));
        shown += 1;
    }
    list.table.set_count(shown);
    widgets::set_count(&list.count, shown, "volume");
}

fn vol_cells(vol: &VolumeRow) -> Vec<gtk::Widget> {
    let pool = match (&vol.pool, vol.kind.as_str()) {
        (Some(p), kind) => format!("{p} · {kind}"),
        (None, kind) => kind.to_string(),
    };
    let actions = widgets::actions_cell();
    if vol.removable {
        let del = widgets::icon_button("user-trash-symbolic", "Remove");
        let (host, engine, name, pool) = (vol.host, vol.engine, vol.name.clone(), vol.pool.clone());
        let host_label = window::host_name(host);
        del.connect_clicked(move |_| {
            let body = format!("Remove volume {name} from {host_label} {}? This cannot be undone.", engine.label());
            let (name, host_label, pool) = (name.clone(), host_label.clone(), pool.clone());
            widgets::confirm("Remove volume?", &body, "Remove", move || {
                window::toast(&format!("Removing {name} from {host_label}…"));
                worker::remove_volume(host, engine, name.clone(), pool.clone());
            });
        });
        actions.append(&del);
    }
    vec![
        widgets::td(&vol.name),
        widgets::td_dim(&window::host_name(vol.host)),
        widgets::td_dim(vol.engine.label()),
        widgets::td_dim(&pool),
        widgets::td_dim(&vol.size_label),
        actions.upcast(),
    ]
}

fn open_create() {
    let (dialog, card) = widgets::dialog("Create volume", 480);
    let host = Rc::new(RefCell::new(HostId::Local));
    let engine = Rc::new(RefCell::new(Engine::Docker));
    card.append(&widgets::segmented_row(
        "Host",
        "Where the volume will live.",
        vec![("local".into(), "Local".into()), ("remote".into(), window::host_name(HostId::Remote))],
        "local",
        {
            let host = host.clone();
            move |id| *host.borrow_mut() = if id == "remote" { HostId::Remote } else { HostId::Local }
        },
    ));
    card.append(&widgets::segmented_row(
        "Engine",
        "Docker named volume, or an Incus custom volume.",
        vec![("docker".into(), "Docker".into()), ("incus".into(), "Incus".into())],
        "docker",
        {
            let engine = engine.clone();
            move |id| *engine.borrow_mut() = if id == "incus" { Engine::Incus } else { Engine::Docker }
        },
    ));
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Name"));
    name.set_hexpand(true);
    card.append(&widgets::row("Name", "", Some(name.upcast_ref())));
    let pool = gtk::Entry::new();
    pool.set_placeholder_text(Some("default"));
    pool.set_hexpand(true);
    card.append(&widgets::row("Pool", "Incus storage pool. Ignored for Docker.", Some(pool.upcast_ref())));
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
        if n.is_empty() {
            window::toast("A volume name is required.");
            return;
        }
        let host = *host.borrow();
        let engine = *engine.borrow();
        if host == HostId::Remote && !prefs::get().remote.configured() {
            window::toast("Configure a remote host first.");
            return;
        }
        let pool_text = pool.text().trim().to_string();
        let pool = if engine == Engine::Incus {
            Some(if pool_text.is_empty() { "default".into() } else { pool_text })
        } else {
            None
        };
        window::toast(&format!("Creating {n} on {}…", window::host_name(host)));
        worker::create_volume(VolumeSpec { host, engine, name: n, pool });
        d.close();
    });
    dialog.present();
}

