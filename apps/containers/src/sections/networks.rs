use crate::models::{Engine, HostFilter, HostId, NetRow, NetSpec, Snapshot};
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
        widgets::col("driver", "Driver", false, false, 96),
        widgets::col("subnet", "Subnet", false, true, 140),
        widgets::col("actions", "", false, true, 48),
    ]
}

pub fn build(page: &Page) {
    page.fill();
    let (bar, count, actions) = widgets::page_header();
    let create = widgets::primary_button("list-add-symbolic", "New network");
    create.connect_clicked(|_| open_create());
    actions.append(&create);
    page.body.append(&bar);
    let table = OpsTable::new(
        &cols(),
        "network-wired-symbolic",
        "No networks",
        "Nothing matches this host filter. Create a bridge network to add one.",
    );
    page.body.append(&table.root);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { table, count })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(list) = UI.with(|u| u.borrow().clone()) else { return };
    list.table.clear();
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
        let row = list.table.push(net_cells(net));
        row.set_tooltip_text(Some(&format!("{} attached", net.used_by)));
        shown += 1;
    }
    list.table.set_count(shown);
    widgets::set_count(&list.count, shown, "network");
}

fn net_cells(net: &NetRow) -> Vec<gtk::Widget> {
    let subnet = net.subnet.as_deref().unwrap_or("—");
    let actions = widgets::actions_cell();
    if net.removable {
        let del = widgets::icon_button("user-trash-symbolic", "Remove");
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
    vec![
        widgets::td(&net.name),
        widgets::td_dim(&window::host_name(net.host)),
        widgets::td_dim(net.engine.label()),
        widgets::td_dim(&net.driver),
        widgets::td_dim(subnet),
        actions.upcast(),
    ]
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

