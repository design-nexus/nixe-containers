use crate::models::{Action, HostFilter, Snapshot, Stack};
use crate::widgets::{self, Page};
use crate::{window, worker};
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
    page.body.add_css_class("overview-page");
    let rows = widgets::vbox(0);
    let empty = widgets::empty_state(
        "view-list-symbolic",
        "No Compose stacks",
        "Projects show up from the com.docker.compose.project label on Docker containers.",
        None,
    );
    page.body.append(&rows);
    page.body.append(&empty);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { rows, empty })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    while let Some(c) = ui.rows.first_child() {
        ui.rows.remove(&c);
    }
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    for stack in snapshot.stacks(host) {
        if !q.is_empty() && !stack.name.to_ascii_lowercase().contains(&q) {
            continue;
        }
        ui.rows.append(&stack_row(&stack));
        shown += 1;
    }
    ui.rows.set_visible(shown > 0);
    ui.empty.set_visible(shown == 0);
}

fn stack_row(stack: &Stack) -> gtk::Box {
    let row = widgets::hbox(8);
    row.add_css_class("workload-row");
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    if stack.running > 0 {
        dot.add_css_class("ok");
    }
    row.append(&dot);
    let name = widgets::label(&stack.name, "");
    name.set_hexpand(true);
    name.set_xalign(0.0);
    row.append(&name);
    row.append(&widgets::label(&window::host_name(stack.host), "dim"));
    row.append(&widgets::label(&format!("{}/{}", stack.running, stack.total), "dim"));
    let actions = widgets::hbox(4);
    actions.set_halign(gtk::Align::End);
    let host = stack.host;
    let names = stack.names.clone();
    if stack.running < stack.total {
        let up = gtk::Button::with_label("Up");
        let n = names.clone();
        up.connect_clicked(move |_| worker::stack_op(host, n.clone(), Action::Start));
        actions.append(&up);
    }
    if stack.running > 0 {
        let down = gtk::Button::with_label("Down");
        let n = names.clone();
        down.connect_clicked(move |_| worker::stack_op(host, n.clone(), Action::Stop));
        actions.append(&down);
        let restart = gtk::Button::with_label("Restart");
        let n = names.clone();
        restart.connect_clicked(move |_| worker::stack_op(host, n.clone(), Action::Restart));
        actions.append(&restart);
    }
    let logs = gtk::Button::with_label("Logs");
    let n = names.clone();
    let project = stack.name.clone();
    logs.connect_clicked(move |_| {
        crate::sections::session::open_stack_logs(host, crate::models::Engine::Docker, &project, n.clone())
    });
    actions.append(&logs);
    row.append(&actions);
    row
}
