use crate::models::{Engine, HostFilter, HostId, Snapshot, VolumeRow, VolumeSpec};
use crate::widgets::{self, Page};
use crate::{prefs, window, worker};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct List {
    rows: gtk::Box,
    empty: gtk::Box,
}

thread_local! {
    static UI: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
}

pub fn build(page: &Page) {
    page.fill();
    let bar = widgets::hbox(8);
    bar.add_css_class("page-toolbar");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let create = gtk::Button::with_label("Create");
    create.add_css_class("suggested-action");
    create.connect_clicked(|_| open_create());
    bar.append(&create);
    page.body.append(&bar);
    let header = row(["Name", "Host", "Engine", "Pool / driver", "Size"], true);
    header.add_css_class("workload-header");
    page.body.append(&header);
    let rows = widgets::vbox(0);
    rows.set_vexpand(true);
    let empty = widgets::empty_state(
        "drive-harddisk-symbolic",
        "No volumes",
        "Nothing matches this host filter. Create a volume to add one.",
        None,
    );
    empty.set_visible(false);
    page.body.append(&rows);
    page.body.append(&empty);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { rows, empty })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(list) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = list.rows.first_child() {
        list.rows.remove(&c);
    }
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
        list.rows.append(&vol_row(vol));
        shown += 1;
    }
    list.rows.set_visible(shown > 0);
    list.empty.set_visible(shown == 0);
}

fn vol_row(vol: &VolumeRow) -> gtk::Box {
    let pool = match (&vol.pool, vol.kind.as_str()) {
        (Some(p), kind) => format!("{p} · {kind}"),
        (None, kind) => kind.to_string(),
    };
    let root = row([&vol.name, &window::host_name(vol.host), vol.engine.label(), &pool, &vol.size_label], false);
    root.add_css_class("workload-row");
    root.set_tooltip_text(Some(&format!("{} attached", vol.used_by)));
    let actions = widgets::hbox(4);
    actions.add_css_class("col-actions");
    if vol.removable {
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.add_css_class("icon-button");
        del.set_tooltip_text(Some("Remove"));
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
    root.append(&actions);
    root
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
