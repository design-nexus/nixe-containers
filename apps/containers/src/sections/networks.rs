use crate::models::{Engine, HostFilter, HostId, NetRow, NetSpec, Snapshot};
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
    let header = row(["Name", "Host", "Engine", "Driver", "Subnet"], true);
    header.add_css_class("workload-header");
    page.body.append(&header);
    let rows = widgets::vbox(0);
    rows.set_vexpand(true);
    let empty = widgets::empty_state(
        "network-wired-symbolic",
        "No networks",
        "Nothing matches this host filter. Create a bridge network to add one.",
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
    for net in snapshot.networks(host) {
        if !q.is_empty()
            && !net.name.to_ascii_lowercase().contains(&q)
            && !net.engine.label().to_ascii_lowercase().contains(&q)
            && !net.driver.to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        list.rows.append(&net_row(net));
        shown += 1;
    }
    list.rows.set_visible(shown > 0);
    list.empty.set_visible(shown == 0);
}

fn net_row(net: &NetRow) -> gtk::Box {
    let subnet = net.subnet.as_deref().unwrap_or("—");
    let root = row([&net.name, &window::host_name(net.host), net.engine.label(), &net.driver, subnet], false);
    root.add_css_class("workload-row");
    root.set_tooltip_text(Some(&format!("{} attached", net.used_by)));
    let actions = widgets::hbox(4);
    actions.add_css_class("col-actions");
    if net.removable {
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.add_css_class("icon-button");
        del.set_tooltip_text(Some("Remove"));
        let (host, engine, name) = (net.host, net.engine, net.name.clone());
        let host_label = window::host_name(host);
        del.connect_clicked(move |_| {
            let body = format!("Remove network {name} from {host_label} {}? This cannot be undone.", engine.label());
            let (name, host_label) = (name.clone(), host_label.clone());
            widgets::confirm("Remove network?", &body, "Remove", move || {
                window::toast(&format!("Removing {name} from {host_label}…"));
                worker::remove_network(host, engine, name.clone());
            });
        });
        actions.append(&del);
    }
    root.append(&actions);
    root
}

fn open_create() {
    let (dialog, card) = widgets::dialog("Create network", 480);
    let host = Rc::new(RefCell::new(HostId::Local));
    let engine = Rc::new(RefCell::new(Engine::Docker));
    card.append(&widgets::segmented_row(
        "Host",
        "Where the network will live.",
        vec![("local".into(), "Local".into()), ("remote".into(), window::host_name(HostId::Remote))],
        "local",
        {
            let host = host.clone();
            move |id| *host.borrow_mut() = if id == "remote" { HostId::Remote } else { HostId::Local }
        },
    ));
    card.append(&widgets::segmented_row(
        "Engine",
        "Docker or Incus bridge.",
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
    let driver = gtk::Entry::new();
    driver.set_placeholder_text(Some("bridge"));
    driver.set_hexpand(true);
    card.append(&widgets::row("Driver", "Docker driver or Incus type. Defaults to bridge.", Some(driver.upcast_ref())));
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
            window::toast("A network name is required.");
            return;
        }
        let host = *host.borrow();
        let engine = *engine.borrow();
        if host == HostId::Remote && !prefs::get().remote.configured() {
            window::toast("Configure a remote host first.");
            return;
        }
        let mut driver = driver.text().trim().to_string();
        if driver.is_empty() {
            driver = "bridge".into();
        }
        window::toast(&format!("Creating {n} on {}…", window::host_name(host)));
        worker::create_network(NetSpec { host, engine, name: n, driver });
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
