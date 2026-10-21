//! Nixe-style building blocks: pages, groups and option rows.

pub use nixe_ui::widgets::*;

use gtk::prelude::*;

pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
}

pub fn page(section: &str) -> Page {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("settings-page");
    body.add_css_class(&format!("page-{section}"));
    body.set_hexpand(true);

    let root = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&body)
        .vexpand(true)
        .build();
    Page { root, body }
}

impl Page {
    pub fn fill(&self) {
        self.root.set_vscrollbar_policy(gtk::PolicyType::Never);
        self.body.set_vexpand(true);
        self.body.add_css_class("fill");
    }

    pub fn group(&self, title: &str) -> Group {
        let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wrapper.add_css_class("settings-group");
        wrapper.set_widget_name(title);
        if !title.is_empty() {
            let l = gtk::Label::new(Some(&title.to_uppercase()));
            l.add_css_class("group-title");
            l.set_xalign(0.0);
            wrapper.append(&l);
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("group-list");
        wrapper.append(&list);
        self.body.append(&wrapper);
        Group { wrapper, list }
    }
}

#[derive(Clone)]
pub struct Group {
    pub wrapper: gtk::Box,
    pub list: gtk::Box,
}

/// Ask before an action that cannot be undone. `body` must name the target host.
pub fn confirm(title: &str, body: &str, action: &str, on_ok: impl Fn() + 'static) {
    let (dialog, card) = dialog(title, 440);
    let desc = label(body, "dim");
    desc.set_wrap(true);
    desc.set_max_width_chars(48);
    card.append(&desc);
    let buttons = hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label(action);
    ok.add_css_class("destructive-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    let d = dialog.clone();
    ok.connect_clicked(move |_| {
        d.close();
        on_ok();
    });
    dialog.present();
}

pub fn sparkline(points: &[f64]) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(36);
    area.set_hexpand(true);
    area.add_css_class("sparkline");
    let pts = points.to_vec();
    area.set_draw_func(move |_, cr, w, h| {
        if w <= 0 || h <= 0 {
            return;
        }
        let pal = crate::theme::palette();
        let color = gtk::gdk::RGBA::parse(&pal.accent).unwrap_or(gtk::gdk::RGBA::new(0.4, 0.6, 1.0, 1.0));
        let danger = gtk::gdk::RGBA::parse(&pal.danger).unwrap_or(gtk::gdk::RGBA::new(0.9, 0.3, 0.3, 1.0));
        let last = pts.last().copied().unwrap_or(0.0);
        let c = if last >= 0.9 { danger } else { color };
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.9);
        if pts.len() < 2 {
            return;
        }
        let n = (pts.len() - 1) as f64;
        cr.set_line_width(1.5);
        cr.move_to(0.0, h as f64 - pts[0].clamp(0.0, 1.0) * (h as f64 - 2.0) - 1.0);
        for (i, v) in pts.iter().enumerate().skip(1) {
            let x = i as f64 / n * w as f64;
            let y = h as f64 - v.clamp(0.0, 1.0) * (h as f64 - 2.0) - 1.0;
            cr.line_to(x, y);
        }
        let _ = cr.stroke();
    });
    area
}

impl Group {
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.list.append(w);
    }

    pub fn note(&self, text: &str) {
        let l = gtk::Label::new(None);
        l.set_markup(text);
        l.add_css_class("group-note");
        l.set_xalign(0.0);
        l.set_wrap(true);
        self.wrapper.insert_child_after(&l, self.wrapper.first_child().as_ref());
    }
}
