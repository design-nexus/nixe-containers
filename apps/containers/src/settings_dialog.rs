//! Settings as a card over the window.

use crate::sections::settings;
use crate::{widgets, window};
use gtk::prelude::*;
use nyx_ui::settings_card::{self, Spec};

pub use nyx_ui::settings_card::{escape, fit, focus_search, is_open};

const GROUPS: &[(&str, &str, &str)] = &[
    ("Diagnostics", "tools-check-spelling-symbolic", "Daemons, groups, packages"),
    ("Containers window", "applications-graphics-symbolic", "Theme, glow, motion"),
    ("Keyboard", "input-keyboard-symbolic", "Shortcuts"),
];

const SPEC: Spec = Spec { groups: GROUPS, search_text: None };

pub fn open() {
    open_to(None);
}

pub fn open_diagnostics() {
    open_to(Some("Diagnostics"));
}

fn open_to(group: Option<&str>) {
    let (Some(overlay), Some(window)) = (window::overlay(), window::window()) else { return };
    settings_card::open(&overlay, &window, SPEC, || {
        let page = widgets::page("settings");
        settings::build(&page);
        (page.root.upcast(), page.body)
    });
    if let Some(name) = group {
        let overlay = overlay.clone();
        let name = name.to_string();
        gtk::glib::idle_add_local_once(move || {
            click_nav(&overlay.upcast(), &name);
        });
    }
}

fn click_nav(widget: &gtk::Widget, name: &str) -> bool {
    if widget.has_css_class("dialog-nav")
        && let Ok(btn) = widget.clone().downcast::<gtk::Button>()
        && nav_title(&btn).as_deref() == Some(name)
    {
        btn.emit_clicked();
        return true;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if click_nav(&c, name) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

fn nav_title(btn: &gtk::Button) -> Option<String> {
    let mut stack = vec![btn.child()?];
    while let Some(w) = stack.pop() {
        if w.has_css_class("dialog-nav-title")
            && let Ok(l) = w.clone().downcast::<gtk::Label>()
        {
            return Some(l.text().to_string());
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            stack.push(ch.clone());
            c = ch.next_sibling();
        }
    }
    None
}
