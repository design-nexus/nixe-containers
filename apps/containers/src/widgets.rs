//! Nixe-style building blocks: pages, groups and option rows.

pub use nixe_ui::widgets::*;

use gtk::prelude::*;
use sourceview5::prelude::*;

pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
}

pub fn page(section: &str) -> Page {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("ops-page");
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

#[derive(Clone, Copy)]
pub struct Col {
    pub key: &'static str,
    pub title: &'static str,
    pub expand: bool,
    pub end: bool,
    pub width: i32,
}

pub fn col(key: &'static str, title: &'static str, expand: bool, end: bool, width: i32) -> Col {
    Col { key, title, expand, end, width }
}

pub struct OpsTable {
    pub root: gtk::Box,
    header: gtk::Grid,
    list: gtk::ListBox,
    empty: gtk::Box,
    scroller: gtk::ScrolledWindow,
    cols: Vec<Col>,
}

impl OpsTable {
    pub fn new(cols: &[Col], icon: &str, empty_title: &str, empty_body: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("ops-table-wrap");
        root.set_vexpand(true);
        let header = make_grid(cols);
        header.add_css_class("ops-thead");
        for (i, c) in cols.iter().enumerate() {
            let l = label(c.title, "ops-th");
            attach_cell(&header, c, l.upcast_ref(), i as i32);
        }
        root.append(&header);
        let list = gtk::ListBox::new();
        list.add_css_class("ops-table");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_vexpand(true);
        let empty = empty_state(icon, empty_title, empty_body, None);
        empty.add_css_class("ops-empty");
        empty.set_visible(false);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        empty.set_vexpand(true);
        root.append(&scroller);
        root.append(&empty);
        Self { root, header, list, empty, scroller, cols: cols.to_vec() }
    }

    pub fn clear(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
    }

    pub fn push(&self, cells: Vec<gtk::Widget>) -> gtk::ListBoxRow {
        let grid = make_grid(&self.cols);
        for (i, cell) in cells.into_iter().enumerate() {
            if let Some(c) = self.cols.get(i) {
                attach_cell(&grid, c, &cell, i as i32);
            }
        }
        let row = gtk::ListBoxRow::new();
        row.add_css_class("ops-row");
        row.set_child(Some(&grid));
        self.list.append(&row);
        row
    }

    pub fn set_count(&self, n: usize) {
        self.list.set_visible(n > 0);
        self.scroller.set_visible(n > 0);
        self.empty.set_visible(n == 0);
        self.header.set_visible(n > 0);
    }

    pub fn listbox(&self) -> gtk::ListBox {
        self.list.clone()
    }
}

impl Clone for OpsTable {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            header: self.header.clone(),
            list: self.list.clone(),
            empty: self.empty.clone(),
            scroller: self.scroller.clone(),
            cols: self.cols.clone(),
        }
    }
}

fn make_grid(cols: &[Col]) -> gtk::Grid {
    let _ = cols;
    let grid = gtk::Grid::new();
    grid.add_css_class("ops-grid");
    grid.set_column_spacing(0);
    grid.set_hexpand(true);
    grid.set_column_homogeneous(false);
    grid
}

fn attach_cell(grid: &gtk::Grid, col: &Col, child: &gtk::Widget, left: i32) {
    child.add_css_class("ops-td");
    child.add_css_class(&format!("ops-col-{}", col.key));
    child.set_hexpand(col.expand);
    if let Some(l) = child.downcast_ref::<gtk::Label>() {
        l.set_xalign(if col.end { 1.0 } else { 0.0 });
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    if col.width > 0 {
        child.set_size_request(col.width, -1);
    }
    if col.end {
        child.set_halign(gtk::Align::End);
    } else {
        child.set_halign(gtk::Align::Fill);
    }
    grid.attach(child, left, 0, 1, 1);
}

pub fn page_header(title: &str) -> (gtk::Box, gtk::Label, gtk::Box) {
    let bar = hbox(12);
    bar.add_css_class("ops-header");
    let titles = vbox(0);
    let t = label(title, "ops-title");
    t.set_xalign(0.0);
    let count = label("", "ops-count");
    count.set_xalign(0.0);
    titles.append(&t);
    titles.append(&count);
    titles.set_hexpand(true);
    bar.append(&titles);
    let actions = hbox(8);
    actions.set_halign(gtk::Align::End);
    bar.append(&actions);
    (bar, count, actions)
}

pub fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.add_css_class("icon-button");
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b
}

/// GtkSourceView paints from a style scheme, not GTK CSS. Build one from the
/// live Nixe palette so the editor matches the window.
pub fn apply_source_scheme(buffer: &sourceview5::Buffer) {
    let palette = crate::theme::palette();
    thread_local! {
        static LAST: std::cell::RefCell<Option<(crate::theme::Palette, sourceview5::StyleScheme)>> =
            const { std::cell::RefCell::new(None) };
    }
    LAST.with(|last| {
        if let Some((old, scheme)) = last.borrow().as_ref()
            && old == &palette
        {
            buffer.set_style_scheme(Some(scheme));
            return;
        }
        let Some(scheme) = build_source_scheme(&palette) else { return };
        buffer.set_style_scheme(Some(&scheme));
        *last.borrow_mut() = Some((palette.clone(), scheme));
    });
}

fn build_source_scheme(palette: &crate::theme::Palette) -> Option<sourceview5::StyleScheme> {
    let directory = crate::paths::cache_dir().join("sourceview");
    std::fs::create_dir_all(&directory).ok()?;
    let bg = gtk::glib::markup_escape_text(&palette.bg);
    let text = gtk::glib::markup_escape_text(&palette.text);
    let dim = gtk::glib::markup_escape_text(&palette.dim_text);
    let accent = gtk::glib::markup_escape_text(&palette.accent);
    let danger = gtk::glib::markup_escape_text(&palette.danger);
    let highlight = gtk::glib::markup_escape_text(&palette.highlight);
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="nixe-containers-yaml" name="Containers" version="1.0">
  <style name="text" foreground="{text}" background="{bg}"/>
  <style name="line-numbers" foreground="{dim}" background="{bg}"/>
  <style name="current-line-number" foreground="{accent}" background="{bg}"/>
  <style name="current-line" background="{highlight}"/>
  <style name="selection" foreground="{text}" background="{highlight}"/>
  <style name="cursor" foreground="{accent}"/>
  <style name="bracket-match" foreground="{accent}" background="{highlight}"/>
  <style name="def:comment" foreground="{dim}" italic="true"/>
  <style name="def:string" foreground="{accent}"/>
  <style name="def:special-char" foreground="{accent}"/>
  <style name="def:keyword" foreground="{accent}" bold="true"/>
  <style name="def:statement" foreground="{text}" bold="true"/>
  <style name="def:identifier" foreground="{text}"/>
  <style name="def:decimal" foreground="{accent}"/>
  <style name="def:floating-point" foreground="{accent}"/>
  <style name="def:constant" foreground="{accent}"/>
  <style name="def:boolean" foreground="{accent}"/>
  <style name="def:type" foreground="{accent}"/>
  <style name="def:function" foreground="{text}"/>
  <style name="def:operator" foreground="{dim}"/>
  <style name="def:preprocessor" foreground="{accent}"/>
  <style name="def:error" foreground="{danger}"/>
</style-scheme>
"#
    );
    crate::cmd::atomic_write(&directory.join("nixe-containers-yaml.xml"), xml.as_bytes()).ok()?;
    let manager = sourceview5::StyleSchemeManager::new();
    manager.append_search_path(&directory.to_string_lossy());
    manager.force_rescan();
    manager.scheme("nixe-containers-yaml")
}

pub fn status_pill(text: &str, kind: &str) -> gtk::Label {
    let l = tag(text, kind);
    l.add_css_class("ops-pill");
    l
}

pub fn td(text: &str) -> gtk::Widget {
    label(text, "").upcast()
}

pub fn td_dim(text: &str) -> gtk::Widget {
    label(text, "dim").upcast()
}

pub fn actions_cell() -> gtk::Box {
    let b = hbox(2);
    b.add_css_class("ops-actions");
    b.set_halign(gtk::Align::End);
    b
}

pub fn kv_line(key: &str, value: &str) -> gtk::Box {
    let row = hbox(8);
    row.add_css_class("ops-kv");
    let k = label(key, "ops-kv-key");
    k.set_xalign(0.0);
    let v = label(value, "");
    v.set_xalign(0.0);
    v.set_hexpand(true);
    v.set_ellipsize(gtk::pango::EllipsizeMode::End);
    v.set_selectable(true);
    row.append(&k);
    row.append(&v);
    row
}

pub fn set_count(label: &gtk::Label, n: usize, noun: &str) {
    let text = if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") };
    label.set_text(&text);
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
