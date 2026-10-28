//! The main window: the header (the app and the page, the sidebar toggle; which hosts
//! to show in the middle; what's running, search, refresh and settings at the end) and
//! the pages in a Nyx sidebar.

use crate::models::{
    DiagReport, HostFilter, HostHealth, HostId, ImageDetail, InstanceSnap, LogChunk, ResourceSample, Snapshot, WorkloadDetail,
};
use crate::sections::{self, Section};
use crate::{prefs, settings_dialog, theme, widgets, worker};
use gtk::prelude::*;
use gtk::{gdk, glib};
use nyx_ui::chrome;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_items: HashMap<String, gtk::Button>,
    search: gtk::SearchEntry,
    pages: HashMap<String, gtk::Widget>,
    sections: Vec<Section>,
    current: String,
    host: HostFilter,
    host_all: gtk::ToggleButton,
    host_local: gtk::ToggleButton,
    host_remote: gtk::ToggleButton,
    overlay: gtk::Overlay,
    header: chrome::AppHeader,
    /// What's running, at the header's end.
    readout: gtk::Label,
    snapshot: Snapshot,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TOAST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

pub fn overlay() -> Option<gtk::Overlay> {
    ui().map(|u| u.borrow().overlay.clone())
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if let Some(ui) = ui() {
        let window = ui.borrow().window.clone();
        if let Some(s) = section {
            navigate(s);
        }
        window.present();
        return;
    }
    theme::install();
    install_icons();
    build(app);
    let start = section.map(String::from).unwrap_or_else(|| nyx_theme::start_page("overview", &prefs::get().last_section));
    let start = match start.as_str() {
        "host-all" | "host-local" | "host-remote" => "overview".to_string(),
        _ => start,
    };
    navigate(&start);
    if prefs::take_broken() {
        toast("Your settings file couldn't be read, so defaults are in use. The old file is kept as settings.toml.bak.");
    }
    worker::start();
    if let Some(ui) = ui() {
        ui.borrow().window.present();
    }
}

fn install_icons() {
    const ICONS: &[(&str, &str)] = &[(
        "io.github.design_nexus.nyx.Containers.svg",
        include_str!("../data/icons/io.github.design_nexus.nyx.Containers.svg"),
    )];
    let dir = crate::paths::cache_dir().join("icons");
    for (name, svg) in ICONS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = crate::cmd::atomic_write(&path, svg);
        }
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let l = widgets::label(title, "nav-label");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&l);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, l)
}

fn build(app: &gtk::Application) {
    let window =
        gtk::ApplicationWindow::builder().application(app).title("Containers").default_width(1280).default_height(800).build();
    window.add_css_class("containers-window");
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common { settings: Some(settings_dialog::open), shortcuts: Some(show_shortcuts), ..Default::default() },
    );
    window.set_icon_name(Some("io.github.design_nexus.nyx.Containers"));

    let sections = sections::all();

    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_hexpand(false);
    let mut nav_items = HashMap::new();
    let mut last_group = "";
    for s in sections.iter().filter(|s| s.nav) {
        if s.group != last_group {
            let g = widgets::label(&s.group.to_uppercase(), "nav-group");
            if last_group.is_empty() {
                g.add_css_class("first");
            }
            g.add_css_class("compact-hide");
            list.append(&g);
            last_group = s.group;
        }
        let (button, label) = nav_button(s.icon, s.title, s.description);
        label.add_css_class("compact-hide");
        let id = s.id;
        button.connect_clicked(move |_| navigate(id));
        list.append(&button);
        nav_items.insert(s.id.to_string(), button);
    }
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hexpand(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    let stack = gtk::Stack::new();
    stack.add_css_class("ops-stack");
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.add_css_class("ops-chrome");
    content.append(&stack);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.append(&nav);
    body.append(&content);

    // ----- Header -----
    // The app and the page, the sidebar toggle; which hosts to show in the middle;
    // what's running, search, refresh and Settings at the end.
    let header = chrome::app_header(&window, "package-x-generic-symbolic", "Containers");
    let collapse = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse);
    let (host_bar, host_all, host_local, host_remote) = host_segment();
    header.center.append(&host_bar);
    header.end.set_spacing(4);
    let readout = widgets::label("Looking for engines…", "header-status");
    readout.set_ellipsize(gtk::pango::EllipsizeMode::End);
    readout.set_margin_end(6);
    header.end.append(&readout);
    let search = search_entry();
    header.end.append(&search);
    let refresh = widgets::bar_button("view-refresh-symbolic", "Refresh the inventory (Ctrl+R)");
    refresh.connect_clicked(|_| worker::refresh());
    header.end.append(&refresh);
    header.end.append(&chrome::settings_button(settings_dialog::open));

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("window-frame");
    frame.append(&header.root);
    frame.append(&body);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));

    install_keys(&window, &search);

    let apply_width = {
        let nav = nav.clone();
        move |w: &gtk::ApplicationWindow| {
            let width = if w.width() > 0 { w.width() } else { w.default_width() };
            let narrow = width > 0 && width < 980;
            settings_dialog::fit(w);
            if narrow == NARROW.with(|n| n.get()) && nav.has_css_class("sized") {
                return;
            }
            nav.add_css_class("sized");
            set_narrow(narrow);
            apply_compact(&nav, narrow || prefs::get().sidebar_collapsed);
        }
    };
    let aw = apply_width.clone();
    window.connect_default_width_notify(move |w| aw(w));
    let aw = apply_width.clone();
    window.connect_realize(move |w| aw(w));
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    probe.set_can_focus(false);
    overlay.add_overlay(&probe);
    overlay.set_measure_overlay(&probe, false);
    {
        let (aw, w2) = (apply_width.clone(), window.clone());
        probe.connect_resize(move |_, _, _| {
            let (aw, w2) = (aw.clone(), w2.clone());
            glib::idle_add_local_once(move || aw(&w2));
        });
    }
    let w2 = window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
        apply_width(&w2);
        glib::ControlFlow::Continue
    });

    let ui = Ui {
        window,
        stack,
        nav,
        nav_items,
        search,
        pages: HashMap::new(),
        sections,
        current: String::new(),
        host: HostFilter::All,
        host_all,
        host_local,
        host_remote,
        overlay,
        header,
        readout,
        snapshot: Snapshot::default(),
    };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));
}

fn search_entry() -> gtk::SearchEntry {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search  (Ctrl+F)"));
    search.add_css_class("bar-search");
    search.set_width_chars(SEARCH_WIDE);
    search.update_property(&[gtk::accessible::Property::Label("Search the current page")]);
    search.connect_search_changed(|_| paint_lists());
    search.connect_stop_search(|e| {
        e.set_text("");
    });
    search
}

fn host_segment() -> (gtk::Box, gtk::ToggleButton, gtk::ToggleButton, gtk::ToggleButton) {
    let bx = widgets::hbox(0);
    bx.add_css_class("segmented");
    bx.add_css_class("host-segment");
    bx.set_valign(gtk::Align::Center);
    let mk = |filter: HostFilter, title: &str| {
        let b = gtk::ToggleButton::with_label(title);
        b.add_css_class("segment");
        b.connect_toggled(move |b| {
            if b.is_active() {
                set_host(filter);
            }
        });
        b
    };
    let all = mk(HostFilter::All, "All");
    let local = mk(HostFilter::Local, "Local");
    let remote = mk(HostFilter::Remote, "Remote");
    local.set_group(Some(&all));
    remote.set_group(Some(&all));
    all.set_active(true);
    remote.set_sensitive(prefs::get().remote.configured());
    remote.set_tooltip_text(Some("The configured remote server"));
    bx.append(&all);
    bx.append(&local);
    bx.append(&remote);
    (bx, all, local, remote)
}

pub fn set_host(filter: HostFilter) {
    let Some(ui) = ui() else { return };
    let current = ui.borrow().host;
    if current == filter {
        return;
    }
    if filter == HostFilter::Remote && !crate::services::connections::remote_ready(&prefs::get().remote) {
        toast("Add a remote on Connections.");
        match current {
            HostFilter::All => ui.borrow().host_all.set_active(true),
            HostFilter::Local => ui.borrow().host_local.set_active(true),
            HostFilter::Remote => {}
        }
        show_page("connections");
        return;
    }
    {
        let mut u = ui.borrow_mut();
        u.host = filter;
        match filter {
            HostFilter::All => u.host_all.set_active(true),
            HostFilter::Local => u.host_local.set_active(true),
            HostFilter::Remote => u.host_remote.set_active(true),
        }
    }
    paint_lists();
}

/// The search's width in characters, in a full window and a narrow one.
const SEARCH_WIDE: i32 = 18;
const SEARCH_NARROW: i32 = 10;

/// Narrow windows (a half-screen tile) get a shorter search and no running counts in
/// the header.
fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    let Some(ui) = ui() else { return };
    let u = ui.borrow();
    u.search.set_width_chars(if narrow { SEARCH_NARROW } else { SEARCH_WIDE });
    u.readout.set_visible(!narrow);
}

fn apply_compact(nav: &gtk::Box, compact: bool) {
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    set_compact_hidden(nav, compact);
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn set_compact_hidden(root: &gtk::Box, compact: bool) {
    fn walk(w: &gtk::Widget, compact: bool) {
        if w.has_css_class("compact-hide") {
            w.set_visible(!compact);
        }
        if w.has_css_class("nav-item")
            && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(root.upcast_ref(), compact);
}

fn install_keys(window: &gtk::ApplicationWindow, search: &gtk::SearchEntry) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s2 = search.clone();
    let w2 = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let typing = gtk::prelude::GtkWindowExt::focus(&w2).is_some_and(|f| {
            f.is::<gtk::Text>() || f.ancestor(gtk::Entry::static_type()).is_some() || f.is::<gtk::SearchEntry>()
        });
        let done = glib::Propagation::Stop;
        if settings_dialog::is_open() {
            return match key {
                gdk::Key::Escape => {
                    settings_dialog::escape();
                    done
                }
                gdk::Key::f if ctrl => {
                    settings_dialog::focus_search();
                    done
                }
                gdk::Key::q | gdk::Key::w if ctrl => {
                    w2.close();
                    done
                }
                _ => glib::Propagation::Proceed,
            };
        }
        match key {
            gdk::Key::f if ctrl => {
                s2.grab_focus();
                done
            }
            gdk::Key::F1 => {
                show_shortcuts();
                done
            }
            gdk::Key::b if ctrl => {
                toggle_sidebar();
                done
            }
            gdk::Key::q | gdk::Key::w if ctrl => {
                w2.close();
                done
            }
            gdk::Key::question if !ctrl && !typing => {
                show_shortcuts();
                done
            }
            gdk::Key::r if ctrl => {
                worker::refresh();
                done
            }
            gdk::Key::Escape if !s2.text().is_empty() => {
                s2.set_text("");
                done
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);
}

pub fn navigate(id: &str) {
    show_page(id);
}

fn show_page(id: &str) {
    let Some(ui) = ui() else { return };
    {
        let mut u = ui.borrow_mut();
        if !u.pages.contains_key(id) {
            let Some(section) = u.sections.iter().find(|s| s.id == id && !s.host_filter).cloned() else { return };
            let page = widgets::page(section.id);
            if section.fill {
                page.fill();
            }
            (section.build)(&page);
            u.stack.add_named(&page.root, Some(section.id));
            u.pages.insert(id.to_string(), page.root.clone().upcast());
        }
        u.stack.set_visible_child_name(id);
        u.current = id.to_string();
        let title = u.sections.iter().find(|s| s.id == id).map(|s| s.title).unwrap_or("Containers");
        chrome::set_title(&u.window, "Containers", title);
        u.header.set_subtitle(title);
        if id != "connections" {
            prefs::update(|p| p.last_section = id.to_string());
        }
    }
    highlight_nav(&ui);
    paint_lists();
}

fn highlight_nav(ui: &Rc<RefCell<Ui>>) {
    let u = ui.borrow();
    for (id, button) in &u.nav_items {
        if id == &u.current {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    }
}

fn paint_lists() {
    let Some(ui) = ui() else { return };
    let u = ui.borrow();
    let query = u.search.text().to_string();
    crate::sections::overview::refresh(&u.snapshot, u.host);
    crate::sections::workloads::refresh(&u.snapshot, u.host, &query);
    crate::sections::images::refresh(&u.snapshot, u.host, &query);
    crate::sections::stacks::refresh(&u.snapshot, u.host, &query);
    crate::sections::networks::refresh(&u.snapshot, u.host, &query);
    crate::sections::storage::refresh(&u.snapshot, u.host, &query);
    crate::sections::connections::refresh_local(&u.snapshot);
}

pub fn host_name(host: HostId) -> String {
    match host {
        HostId::Local => "Local".into(),
        HostId::Remote => {
            let n = prefs::get().remote.display_name();
            if n.is_empty() { "Remote".into() } else { n }
        }
    }
}

pub fn host_filter() -> HostFilter {
    ui().map(|u| u.borrow().host).unwrap_or(HostFilter::All)
}

pub fn snapshot() -> Option<Snapshot> {
    ui().map(|u| u.borrow().snapshot.clone())
}

pub fn apply_health(list: Vec<HostHealth>) {
    crate::sections::overview::apply_health(list);
}

pub fn apply_diag(report: DiagReport) {
    crate::sections::diagnose::apply(report);
}

pub fn apply_fix(result: Result<String, String>) {
    match result {
        Ok(msg) => {
            toast(&msg);
            worker::diagnose();
            worker::refresh();
        }
        Err(err) => {
            if let Some(cmd) = err.lines().last() {
                copy_text(cmd);
            }
            toast(&err);
        }
    }
}

fn copy_text(text: &str) {
    if let Some(display) = gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

pub fn apply_snapshot(snapshot: Snapshot) {
    let Some(ui) = ui() else { return };
    {
        let mut u = ui.borrow_mut();
        let running = snapshot.running(HostFilter::All, None);
        let stopped = snapshot.stopped(HostFilter::All, None);
        let connected = snapshot.engines.iter().filter(|e| e.is_connected()).count();
        u.readout.set_text(&format!("{running} running · {stopped} stopped"));
        u.readout.set_tooltip_text(Some(&format!(
            "{running} running · {stopped} stopped · {connected} {} connected",
            if connected == 1 { "engine" } else { "engines" }
        )));
        u.snapshot = snapshot;
        let remote_ok = crate::services::connections::remote_ready(&prefs::get().remote);
        u.host_remote.set_sensitive(remote_ok);
        if !remote_ok && u.host == HostFilter::Remote {
            u.host = HostFilter::All;
            u.host_all.set_active(true);
        }
    }
    paint_lists();
}

pub fn apply_op(result: Result<String, String>) {
    match result {
        Ok(msg) => {
            toast(&msg);
            worker::refresh();
        }
        Err(err) => toast(&err),
    }
}

pub fn apply_detail(result: Result<WorkloadDetail, String>) {
    match result {
        Ok(detail) => crate::sections::workloads::show_detail(detail),
        Err(err) => toast(&err),
    }
}

pub fn apply_stats(result: Result<ResourceSample, String>) {
    if let Ok(sample) = result {
        crate::sections::workloads::apply_stats(sample);
    }
}

pub fn apply_image_detail(result: Result<ImageDetail, String>) {
    match result {
        Ok(detail) => crate::sections::images::show_detail(detail),
        Err(err) => toast(&err),
    }
}

pub fn apply_snaps(host: HostId, instance: String, result: Result<Vec<InstanceSnap>, String>) {
    match result {
        Ok(snaps) => crate::sections::workloads::apply_snaps(host, &instance, &snaps),
        Err(err) => toast(&err),
    }
}

pub fn apply_log(chunk: LogChunk) {
    crate::sections::session::apply_chunk(chunk);
}

pub fn apply_log_done(seq: u64, result: Result<(), String>) {
    crate::sections::session::apply_done(seq, result);
}

pub fn apply_term_bytes(seq: u64, bytes: Vec<u8>) {
    crate::sections::session::apply_bytes(seq, bytes);
}

pub fn apply_term_done(seq: u64, result: Result<(), String>) {
    crate::sections::session::apply_term_done(seq, result);
}

pub fn apply_test(results: Vec<crate::models::EngineStatus>) {
    crate::sections::connections::show_test(&results);
    let ok = results.iter().filter(|e| e.is_connected()).count();
    let name = crate::services::connections::remote_label(&prefs::get().remote);
    toast(&format!("{name}: {ok} of {} engines connected.", results.len()));
}

pub fn toast(message: &str) {
    let Some(ui) = ui() else {
        eprintln!("nyx-containers: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    if let Some(old) = TOAST.with(|t| t.borrow_mut().take()) {
        overlay.remove_overlay(&old);
    }
    let bx = widgets::hbox(14);
    bx.add_css_class("toast");
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    bx.set_margin_bottom(48);
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(64);
    bx.append(&label);
    overlay.add_overlay(&bx);
    TOAST.with(|t| *t.borrow_mut() = Some(bx.clone()));
    let overlay = overlay.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
        overlay.remove_overlay(&bx);
        TOAST.with(|t| *t.borrow_mut() = None);
    });
}

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "F"], "Search the current page"),
    (&["Esc"], "Clear the search"),
    (&["Ctrl", "R"], "Refresh inventory"),
    (&["Ctrl", "B"], "Collapse or expand the sidebar"),
    (&["Ctrl", ","], "Settings"),
    (&["F1"], "Show these shortcuts (or ?)"),
    (&["Ctrl", "Q"], "Close"),
];

pub fn show_shortcuts() {
    let (dialog, card) = widgets::dialog("Keyboard shortcuts", 520);
    let list = widgets::vbox(0);
    list.add_css_class("group-list");
    for (keys, what) in SHORTCUTS {
        list.append(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&list)
        .build();
    card.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    card.append(&close);
    dialog.present();
}
