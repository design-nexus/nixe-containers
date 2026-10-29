use crate::widgets::Page;

pub fn build(page: &Page) {
    use gtk::prelude::*;
    use crate::{prefs, widgets};
    let p = prefs::get();
    let group = page.group("Refresh");
    let (row, _) = widgets::choice_row("Update every", "How often to refresh containers, images and volumes.",
        vec![("2".into(), "2 seconds".into()), ("4".into(), "4 seconds".into()), ("8".into(), "8 seconds".into()),
             ("15".into(), "15 seconds".into()), ("30".into(), "30 seconds".into())],
        &p.refresh_seconds.to_string(), |id| { prefs::update(|p| p.refresh_seconds = id.parse().unwrap_or(8)); prefs::flush(); });
    group.add(&row);
    let group = page.group("Remote");
    let host = gtk::Entry::new();
    host.set_text(&p.remote.host);
    host.set_width_chars(18);
    host.connect_changed(|e| { prefs::update(|p| p.remote.host = e.text().trim().to_string()); prefs::flush(); });
    group.add(&widgets::row("Host", "Hostname or IP of your remote server.", Some(host.upcast_ref())));
    let name = gtk::Entry::new();
    name.set_text(&p.remote.name);
    name.set_width_chars(18);
    name.connect_changed(|e| { prefs::update(|p| p.remote.name = e.text().to_string()); prefs::flush(); });
    group.add(&widgets::row("Name", "Friendly name shown in the inventory.", Some(name.upcast_ref())));
    let button = gtk::Button::from_icon_name("network-server-symbolic");
    button.set_tooltip_text(Some("Open connection details"));
    button.connect_clicked(|_| { crate::settings_dialog::escape(); crate::window::navigate("connections"); });
    group.add(&widgets::row("Connection details", "Configure Docker SSH, Incus certificates and trust on Connections.", Some(button.upcast_ref())));
    crate::sections::diagnose::build(page);
}
