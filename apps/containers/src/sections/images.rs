use crate::models::{Engine, HostFilter, HostId, Image, ImageDetail, Snapshot};
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

fn cols() -> [Col; 5] {
    [
        widgets::col("name", "Name", true, false, 0),
        widgets::col("host", "Host", false, false, 88),
        widgets::col("engine", "Engine", false, false, 80),
        widgets::col("size", "Size", false, true, 88),
        widgets::col("actions", "", false, true, 72),
    ]
}

pub fn build(page: &Page) {
    page.fill();
    let (bar, count, actions) = widgets::page_header();
    let pull = widgets::primary_button("folder-download-symbolic", "Pull an image");
    pull.connect_clicked(|_| open_pull());
    actions.append(&pull);
    page.body.append(&bar);
    let table = OpsTable::new(
        &cols(),
        "media-optical-symbolic",
        "No images",
        "Nothing matches this host filter. Pull an image to add one.",
    );
    page.body.append(&table.root);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { table, count })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(list) = UI.with(|u| u.borrow().clone()) else { return };
    list.table.clear();
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    for img in snapshot.images(host) {
        if !q.is_empty()
            && !img.name.to_ascii_lowercase().contains(&q)
            && !img.engine.label().to_ascii_lowercase().contains(&q)
            && !img.id.to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        list.table.push(image_cells(img));
        shown += 1;
    }
    list.table.set_count(shown);
    widgets::set_count(&list.count, shown, "image");
}

fn image_cells(img: &Image) -> Vec<gtk::Widget> {
    let actions = widgets::actions_cell();
    let inspect = widgets::icon_button("dialog-information-symbolic", "Inspect");
    let (host, engine, id) = (img.host, img.engine, img.id.clone());
    inspect.connect_clicked(move |_| worker::inspect_image(host, engine, id.clone()));
    let del = widgets::icon_button("user-trash-symbolic", "Remove");
    let (host, engine, id, name) = (img.host, img.engine, img.id.clone(), img.name.clone());
    let host_label = window::host_name(host);
    del.connect_clicked(move |_| {
        let body = format!("Remove image {name} from {host_label} {}? This cannot be undone.", engine.label());
        let (id, name, host_label) = (id.clone(), name.clone(), host_label.clone());
        widgets::confirm("Remove image?", &body, "Remove", move || {
            window::toast(&format!("Removing {name} from {host_label}…"));
            worker::remove_image(host, engine, id.clone());
        });
    });
    actions.append(&inspect);
    actions.append(&del);
    vec![
        widgets::td(&img.name),
        widgets::td_dim(&window::host_name(img.host)),
        widgets::td_dim(img.engine.label()),
        widgets::td_dim(&crate::models::format_bytes(img.size)),
        actions.upcast(),
    ]
}

pub fn show_detail(detail: ImageDetail) {
    let (dialog, card) = widgets::dialog(&detail.image.name, 520);
    card.append(&kv("Host", &window::host_name(detail.image.host)));
    card.append(&kv("Engine", detail.image.engine.label()));
    card.append(&kv("Size", &crate::models::format_bytes(detail.image.size)));
    if let Some(created) = &detail.image.created {
        card.append(&kv("Created", created));
    }
    for (k, v) in &detail.extra {
        card.append(&kv(k, v));
    }
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    dialog.present();
}

fn kv(title: &str, value: &str) -> gtk::Box {
    widgets::row(title, "", Some(widgets::label(value, "").upcast_ref()))
}

fn open_pull() {
    let (dialog, card) = widgets::dialog("Pull image", 480);
    let host = Rc::new(RefCell::new(HostId::Local));
    let engine = Rc::new(RefCell::new(Engine::Docker));
    card.append(&widgets::segmented_row(
        "Host",
        "Where the image will be stored.",
        vec![("local".into(), "Local".into()), ("remote".into(), window::host_name(HostId::Remote))],
        "local",
        {
            let host = host.clone();
            move |id| *host.borrow_mut() = if id == "remote" { HostId::Remote } else { HostId::Local }
        },
    ));
    card.append(&widgets::segmented_row(
        "Engine",
        "Docker Hub tags, or Incus aliases from images.linuxcontainers.org.",
        vec![("docker".into(), "Docker".into()), ("incus".into(), "Incus".into())],
        "docker",
        {
            let engine = engine.clone();
            move |id| *engine.borrow_mut() = if id == "incus" { Engine::Incus } else { Engine::Docker }
        },
    ));
    let reference = gtk::Entry::new();
    reference.set_placeholder_text(Some("nginx:latest"));
    reference.set_hexpand(true);
    card.append(&widgets::row("Image", "", Some(reference.upcast_ref())));
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label("Pull");
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let d = dialog.clone();
    ok.connect_clicked(move |_| {
        let r = reference.text().trim().to_string();
        if r.is_empty() {
            window::toast("An image name is required.");
            return;
        }
        let host = *host.borrow();
        let engine = *engine.borrow();
        if host == HostId::Remote && !prefs::get().remote.configured() {
            window::toast("Configure a remote host first.");
            return;
        }
        window::toast(&format!("Pulling {r} on {}…", window::host_name(host)));
        worker::pull_image(host, engine, r);
        d.close();
    });
    dialog.present();
}

