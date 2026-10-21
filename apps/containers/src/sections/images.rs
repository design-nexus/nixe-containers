use crate::models::{Engine, HostFilter, HostId, Image, ImageDetail, Snapshot};
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
    let pull = gtk::Button::with_label("Pull");
    pull.add_css_class("suggested-action");
    pull.connect_clicked(|_| open_pull());
    bar.append(&pull);
    page.body.append(&bar);
    let header = row(["Name", "Host", "Engine", "Size"], true);
    header.add_css_class("workload-header");
    page.body.append(&header);
    let rows = widgets::vbox(0);
    rows.set_vexpand(true);
    let empty = widgets::empty_state(
        "media-optical-symbolic",
        "No images",
        "Nothing matches this host filter. Pull an image to add one.",
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
    for img in snapshot.images(host) {
        if !q.is_empty()
            && !img.name.to_ascii_lowercase().contains(&q)
            && !img.engine.label().to_ascii_lowercase().contains(&q)
            && !img.id.to_ascii_lowercase().contains(&q)
        {
            continue;
        }
        list.rows.append(&image_row(img));
        shown += 1;
    }
    list.rows.set_visible(shown > 0);
    list.empty.set_visible(shown == 0);
}

fn image_row(img: &Image) -> gtk::Box {
    let root = row([&img.name, &window::host_name(img.host), img.engine.label(), &crate::models::format_bytes(img.size)], false);
    root.add_css_class("workload-row");
    let actions = widgets::hbox(4);
    actions.add_css_class("col-actions");
    let inspect = gtk::Button::from_icon_name("dialog-information-symbolic");
    inspect.add_css_class("flat");
    inspect.add_css_class("icon-button");
    inspect.set_tooltip_text(Some("Inspect"));
    let (host, engine, id) = (img.host, img.engine, img.id.clone());
    inspect.connect_clicked(move |_| worker::inspect_image(host, engine, id.clone()));
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.add_css_class("icon-button");
    del.set_tooltip_text(Some("Remove"));
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
    root.append(&actions);
    root
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

fn row(cols: [&str; 4], header: bool) -> gtk::Box {
    let b = widgets::hbox(12);
    let classes = ["col-name", "col-host", "col-engine", "col-status"];
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
