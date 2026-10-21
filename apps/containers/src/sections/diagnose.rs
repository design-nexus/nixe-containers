//! Settings diagnostics: engine packages, groups, daemons.

use crate::models::{CheckLevel, DiagCheck, DiagReport};
use crate::widgets::{self, Page};
use crate::worker;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Ui {
    list: gtk::Box,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

pub fn build(page: &Page) {
    let g = page.group("Diagnostics");
    g.note("Checks this computer. Fix runs the command with pkexec. If that is cancelled, the command is copied.");
    let list = widgets::vbox(0);
    g.add(&list);
    let refresh = gtk::Button::with_label("Recheck");
    refresh.connect_clicked(|_| worker::diagnose());
    g.add(&refresh);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(Ui { list })));
    worker::diagnose();
}

pub fn apply(report: DiagReport) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = ui.list.first_child() {
        ui.list.remove(&c);
    }
    for check in report.checks {
        ui.list.append(&row(&check));
    }
}

fn row(check: &DiagCheck) -> gtk::Box {
    let kind = match check.level {
        CheckLevel::Ok => "ok",
        CheckLevel::Fail => "danger",
    };
    let status = match check.level {
        CheckLevel::Ok => "OK",
        CheckLevel::Fail => "Fail",
    };
    let extra = if check.level == CheckLevel::Fail {
        let id = check.id;
        let cmd = check.command.clone().unwrap_or_default();
        let box_ = widgets::hbox(8);
        box_.append(&widgets::tag(status, kind));
        if !cmd.is_empty() {
            let fix = gtk::Button::with_label("Fix");
            fix.add_css_class("suggested-action");
            fix.connect_clicked(move |_| worker::fix(id));
            box_.append(&fix);
            let copy = gtk::Button::with_label("Copy");
            let c = cmd.clone();
            copy.connect_clicked(move |_| {
                if let Some(d) = gtk::gdk::Display::default() {
                    d.clipboard().set_text(&c);
                }
                crate::window::toast("Command copied.");
            });
            box_.append(&copy);
        }
        Some(box_.upcast::<gtk::Widget>())
    } else {
        Some(widgets::tag(status, kind).upcast())
    };
    widgets::row(check.id.title(), &check.detail, extra.as_ref())
}
