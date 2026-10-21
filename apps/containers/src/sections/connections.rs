use crate::models::{ConnState, EngineStatus, Snapshot, TrustKind};
use crate::prefs::{self, RemoteProfile};
use crate::services::credentials::{Secret, store_secret};
use crate::widgets::{self, Page};
use crate::{window, worker};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Fields {
    name: gtk::Entry,
    host: gtk::Entry,
    docker_on: gtk::Switch,
    ssh_user: gtk::Entry,
    ssh_port: gtk::Entry,
    identity: gtk::Entry,
    passphrase: gtk::PasswordEntry,
    docker_socket: gtk::Entry,
    incus_on: gtk::Switch,
    incus_url: gtk::Entry,
    incus_cert: gtk::Entry,
    incus_key: gtk::Entry,
    local: gtk::Box,
    test_out: gtk::Box,
}

thread_local! {
    static UI: RefCell<Option<Rc<Fields>>> = const { RefCell::new(None) };
}

pub fn build(page: &Page) {
    let g = page.group("This computer");
    g.note("Engines are detected automatically. Use Diagnostics if a daemon is down or this user lacks a group.");
    let local = widgets::vbox(0);
    g.add(&local);
    let diag = gtk::Button::with_label("Open diagnostics");
    diag.connect_clicked(|_| crate::settings_dialog::open_diagnostics());
    g.add(&diag);

    let g = page.group("Remote server");
    g.note("One host. Docker uses SSH. Incus uses HTTPS with a client certificate. Unknown SSH host keys and TLS certificates must be trusted explicitly.");
    let name = entry("Friendly name");
    let host = entry("Hostname or IP");
    g.add(&widgets::row("Name", "Shown in the sidebar and inventory.", Some(name.upcast_ref())));
    g.add(&widgets::row("Host", "The remote machine.", Some(host.upcast_ref())));

    let docker_on = gtk::Switch::new();
    g.add(&widgets::row("Docker over SSH", "Talk to the remote Docker socket through SSH.", Some(docker_on.upcast_ref())));
    let ssh_user = entry("User");
    let ssh_port = entry("22");
    let identity = entry("~/.ssh/id_ed25519");
    let docker_socket = entry("/var/run/docker.sock");
    g.add(&widgets::row("SSH user", "", Some(ssh_user.upcast_ref())));
    g.add(&widgets::row("SSH port", "", Some(ssh_port.upcast_ref())));
    g.add(&widgets::row("Identity file", "Private key path. Leave empty for the SSH agent.", Some(identity.upcast_ref())));
    let passphrase = gtk::PasswordEntry::new();
    passphrase.set_show_peek_icon(true);
    passphrase.set_hexpand(true);
    g.add(&widgets::row(
        "Key passphrase",
        "Stored in the system keyring, never in settings.toml.",
        Some(passphrase.upcast_ref()),
    ));
    g.add(&widgets::row("Remote socket", "Docker socket on the remote host.", Some(docker_socket.upcast_ref())));

    let incus_on = gtk::Switch::new();
    g.add(&widgets::row("Incus over HTTPS", "Certificate-authenticated Incus API.", Some(incus_on.upcast_ref())));
    let incus_url = entry("https://host:8443");
    let incus_cert = entry("/path/client.crt");
    let incus_key = entry("/path/client.key");
    g.add(&widgets::row("Incus URL", "", Some(incus_url.upcast_ref())));
    g.add(&widgets::row("Client certificate", "", Some(incus_cert.upcast_ref())));
    g.add(&widgets::row("Client key", "", Some(incus_key.upcast_ref())));

    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let test = gtk::Button::with_label("Test connection");
    buttons.append(&test);
    buttons.append(&save);
    g.add(&buttons);

    let test_out = widgets::vbox(0);
    g.add(&test_out);

    save.connect_clicked(|_| {
        save_from_fields();
        window::toast("Remote profile saved.");
        worker::refresh();
    });
    test.connect_clicked(|_| {
        save_from_fields();
        worker::test_remote(prefs::get().remote);
    });

    let fields = Rc::new(Fields {
        name,
        host,
        docker_on,
        ssh_user,
        ssh_port,
        identity,
        passphrase,
        docker_socket,
        incus_on,
        incus_url,
        incus_cert,
        incus_key,
        local,
        test_out,
    });
    load_into(&fields, &prefs::get().remote);
    UI.with(|u| *u.borrow_mut() = Some(fields));
}

fn entry(placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    e.set_width_chars(24);
    e
}

fn load_into(f: &Fields, r: &RemoteProfile) {
    f.name.set_text(&r.name);
    f.host.set_text(&r.host);
    f.docker_on.set_active(r.docker_enabled);
    f.ssh_user.set_text(&r.docker_ssh_user);
    f.ssh_port.set_text(&r.docker_ssh_port.to_string());
    f.identity.set_text(&r.docker_identity_file);
    f.docker_socket.set_text(&r.docker_socket);
    f.incus_on.set_active(r.incus_enabled);
    f.incus_url.set_text(&r.incus_url);
    f.incus_cert.set_text(&r.incus_client_cert);
    f.incus_key.set_text(&r.incus_client_key);
}

fn save_from_fields() {
    let Some(f) = UI.with(|u| u.borrow().clone()) else { return };
    prefs::update(|p| {
        p.remote.name = f.name.text().to_string();
        p.remote.host = f.host.text().to_string();
        p.remote.docker_enabled = f.docker_on.is_active();
        p.remote.docker_ssh_user = f.ssh_user.text().to_string();
        p.remote.docker_ssh_port = f.ssh_port.text().parse().unwrap_or(22);
        p.remote.docker_identity_file = f.identity.text().to_string();
        p.remote.docker_socket = f.docker_socket.text().to_string();
        p.remote.incus_enabled = f.incus_on.is_active();
        p.remote.incus_url = f.incus_url.text().to_string();
        p.remote.incus_client_cert = f.incus_cert.text().to_string();
        p.remote.incus_client_key = f.incus_key.text().to_string();
    });
    prefs::flush();
    let pass = f.passphrase.text().to_string();
    if !pass.is_empty() {
        match Secret::new(pass) {
            Ok(secret) => {
                if let Err(err) = store_secret("ssh-passphrase", &secret) {
                    window::toast(&format!("The keyring could not store the passphrase: {err}"));
                }
            }
            Err(err) => window::toast(&err.to_string()),
        }
        f.passphrase.set_text("");
    }
}

pub fn refresh_local(snapshot: &Snapshot) {
    let Some(f) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = f.local.first_child() {
        f.local.remove(&c);
    }
    for st in snapshot.engines.iter().filter(|e| e.host == crate::models::HostId::Local) {
        f.local.append(&status_row(st, false));
    }
    if f.local.first_child().is_none() {
        f.local.append(&widgets::row("Checking…", "Looking for local Docker and Incus sockets.", None));
    }
}

pub fn show_test(results: &[EngineStatus]) {
    let Some(f) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = f.test_out.first_child() {
        f.test_out.remove(&c);
    }
    for st in results {
        f.test_out.append(&status_row(st, true));
        if let ConnState::Untrusted { fingerprint, kind } = &st.state {
            let fp = fingerprint.clone();
            let kind = *kind;
            let trust = gtk::Button::with_label("Trust this identity");
            trust.add_css_class("suggested-action");
            trust.connect_clicked(move |_| accept_trust(kind, &fp));
            f.test_out.append(&trust);
        }
    }
}

fn accept_trust(kind: TrustKind, fingerprint: &str) {
    prefs::update(|p| match kind {
        TrustKind::Ssh => p.remote.ssh_host_fingerprint = fingerprint.to_string(),
        TrustKind::Tls => p.remote.incus_server_fingerprint = fingerprint.to_string(),
    });
    prefs::flush();
    window::toast("Identity stored. Testing again.");
    worker::test_remote(prefs::get().remote);
    worker::refresh();
}

fn status_row(st: &EngineStatus, _test: bool) -> gtk::Box {
    let (label, desc, _kind) = match &st.state {
        ConnState::Connected { detail } => ("Connected", detail.as_str(), "ok"),
        ConnState::Unavailable { reason } => ("Unavailable", reason.as_str(), "danger"),
        ConnState::Permission { reason } => ("Permission", reason.as_str(), "danger"),
        ConnState::Unconfigured => ("Off", "This engine is not enabled.", ""),
        ConnState::Untrusted { fingerprint, .. } => ("Untrusted", fingerprint.as_str(), "danger"),
        ConnState::Insecure { reason } => ("Blocked", reason.as_str(), "danger"),
    };
    widgets::row(&format!("{} {}", st.host.label(), st.engine.label()), &format!("{label}. {desc}"), None)
}
