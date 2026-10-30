//! Logs and interactive terminal dialogs.

use crate::models::{Engine, HostId, Kind, LogChunk, LogFilter, RunState, TermKind, log_visible};
use crate::widgets;
use crate::{window, worker};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use vte4::prelude::*;

struct LogsUi {
    chunks: RefCell<Vec<LogChunk>>,
    buffer: gtk::TextBuffer,
    view: gtk::TextView,
    query: gtk::SearchEntry,
    filter: Rc<RefCell<LogFilter>>,
    follow: gtk::Switch,
}

thread_local! {
    static LOGS: RefCell<HashMap<u64, Rc<LogsUi>>> = RefCell::new(HashMap::new());
    static TERMS: RefCell<HashMap<u64, vte4::Terminal>> = RefCell::new(HashMap::new());
    static SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn next_seq() -> u64 {
    SEQ.with(|s| {
        let n = s.get() + 1;
        s.set(n);
        n
    })
}

pub fn open_logs(host: HostId, engine: Engine, name: &str) {
    let seq = next_seq();
    let title = format!("Logs · {name} · {}", window::host_name(host));
    let (dialog, card) = widgets::dialog(&title, 720);
    crate::dialog_dismissal::add(&dialog, &card, "Close");
    dialog.set_default_height(520);

    let bar = widgets::hbox(8);
    let query = gtk::SearchEntry::new();
    query.set_placeholder_text(Some("Search logs"));
    query.set_hexpand(true);
    bar.append(&query);
    let follow = gtk::Switch::new();
    follow.set_active(true);
    follow.set_valign(gtk::Align::Center);
    bar.append(&widgets::label("Follow", "dim"));
    bar.append(&follow);
    card.append(&bar);

    let filter = Rc::new(RefCell::new(LogFilter::All));
    if engine == Engine::Docker {
        card.append(&widgets::segmented_row(
            "Stream",
            "",
            vec![("all".into(), "All".into()), ("stdout".into(), "Stdout".into()), ("stderr".into(), "Stderr".into())],
            "all",
            {
                let filter = filter.clone();
                move |id| {
                    *filter.borrow_mut() = match id.as_str() {
                        "stdout" => LogFilter::Stdout,
                        "stderr" => LogFilter::Stderr,
                        _ => LogFilter::All,
                    };
                    LOGS.with(|m| {
                        if let Some(ui) = m.borrow().get(&seq) {
                            rebuild(ui);
                        }
                    });
                }
            },
        ));
    }

    let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::Char);
    view.set_monospace(true);
    view.add_css_class("logs-view");
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .hexpand(true)
        .child(&view)
        .build();
    card.append(&scroll);

    let ui =
        Rc::new(LogsUi { chunks: RefCell::new(Vec::new()), buffer, view, query: query.clone(), filter, follow: follow.clone() });
    let ui_q = ui.clone();
    query.connect_search_changed(move |_| rebuild(&ui_q));
    LOGS.with(|m| {
        m.borrow_mut().insert(seq, ui);
    });

    let follow_name = name.to_string();
    follow.connect_active_notify(move |sw| {
        if sw.is_active() {
            worker::follow_logs(host, engine, vec![follow_name.clone()], seq, true);
        } else {
            worker::stop_logs(seq);
        }
    });
    dialog.connect_close_request(move |_| {
        worker::stop_logs(seq);
        LOGS.with(|m| {
            m.borrow_mut().remove(&seq);
        });
        glib::Propagation::Proceed
    });
    worker::follow_logs(host, engine, vec![name.to_string()], seq, true);
    dialog.present();
}

pub fn open_stack_logs(host: HostId, engine: Engine, project: &str, names: Vec<String>) {
    let seq = next_seq();
    let title = format!("Logs · {project} · {}", window::host_name(host));
    let (dialog, card) = widgets::dialog(&title, 720);
    crate::dialog_dismissal::add(&dialog, &card, "Close");
    dialog.set_default_height(520);
    let bar = widgets::hbox(8);
    let query = gtk::SearchEntry::new();
    query.set_placeholder_text(Some("Search logs"));
    query.set_hexpand(true);
    bar.append(&query);
    let follow = gtk::Switch::new();
    follow.set_active(true);
    follow.set_valign(gtk::Align::Center);
    bar.append(&widgets::label("Follow", "dim"));
    bar.append(&follow);
    card.append(&bar);
    let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let view = gtk::TextView::with_buffer(&buffer);
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::Char);
    view.set_monospace(true);
    view.add_css_class("logs-view");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&view)
        .build();
    card.append(&scroll);
    let filter = Rc::new(RefCell::new(LogFilter::All));
    let ui =
        Rc::new(LogsUi { chunks: RefCell::new(Vec::new()), buffer, view, query: query.clone(), filter, follow: follow.clone() });
    query.connect_search_changed({
        let ui = ui.clone();
        move |_| rebuild(&ui)
    });
    LOGS.with(|m| {
        m.borrow_mut().insert(seq, ui);
    });
    let follow_names = names.clone();
    follow.connect_active_notify(move |sw| {
        if sw.is_active() {
            worker::follow_logs(host, engine, follow_names.clone(), seq, true);
        } else {
            worker::stop_logs(seq);
        }
    });
    dialog.connect_close_request(move |_| {
        worker::stop_logs(seq);
        LOGS.with(|m| {
            m.borrow_mut().remove(&seq);
        });
        glib::Propagation::Proceed
    });
    worker::follow_logs(host, engine, names, seq, true);
    dialog.present();
}

pub fn apply_chunk(chunk: LogChunk) {
    let Some(ui) = LOGS.with(|m| m.borrow().get(&chunk.seq).cloned()) else { return };
    {
        let mut chunks = ui.chunks.borrow_mut();
        chunks.push(chunk);
        if chunks.len() > 8000 {
            let drop = chunks.len() - 8000;
            chunks.drain(..drop);
        }
    }
    rebuild(&ui);
}

pub fn apply_done(_seq: u64, result: Result<(), String>) {
    if let Err(err) = result {
        window::toast(&err);
    }
}

fn rebuild(ui: &LogsUi) {
    let query = ui.query.text().to_string();
    let filter = *ui.filter.borrow();
    let mut text = String::new();
    for chunk in ui.chunks.borrow().iter() {
        if log_visible(chunk, filter, &query) {
            text.push_str(&chunk.text);
            if !chunk.text.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    ui.buffer.set_text(&text);
    if ui.follow.is_active() {
        let mut end = ui.buffer.end_iter();
        ui.view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
    }
}

pub fn open_term(host: HostId, engine: Engine, name: &str, kind: TermKind, running: bool) {
    if !running {
        window::toast("The workload must be running to open a terminal.");
        return;
    }
    let seq = next_seq();
    let kind_label = match kind {
        TermKind::Exec => "Terminal",
        TermKind::Console => "Console",
    };
    let title = format!("{kind_label} · {name} · {}", window::host_name(host));
    let (dialog, card) = widgets::dialog(&title, 760);
    crate::dialog_dismissal::add(&dialog, &card, "Close");
    dialog.set_default_height(480);
    let hint = widgets::label(
        match kind {
            TermKind::Exec => "Interactive exec. Close the window to detach.",
            TermKind::Console => "Instance console. Close the window to detach.",
        },
        "dim",
    );
    hint.set_wrap(true);
    card.append(&hint);
    let term = vte4::Terminal::new();
    term.set_hexpand(true);
    term.set_vexpand(true);
    term.set_scrollback_lines(8000);
    let font = gtk::pango::FontDescription::from_string("JetBrains Mono 11");
    term.set_font(Some(&font));
    term.add_css_class("session-term");
    term.connect_commit(move |_, text, _| {
        worker::term_input(seq, text.as_bytes().to_vec());
    });
    term.connect_notify_local(Some("allocated-width"), move |t, _| {
        worker::term_resize(seq, t.column_count() as u16, t.row_count() as u16);
    });
    term.connect_notify_local(Some("allocated-height"), move |t, _| {
        worker::term_resize(seq, t.column_count() as u16, t.row_count() as u16);
    });
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .hexpand(true)
        .child(&term)
        .build();
    card.append(&scroll);
    TERMS.with(|m| {
        m.borrow_mut().insert(seq, term.clone());
    });
    term.grab_focus();
    dialog.connect_close_request(move |_| {
        worker::stop_term(seq);
        TERMS.with(|m| {
            m.borrow_mut().remove(&seq);
        });
        glib::Propagation::Proceed
    });
    worker::start_term(host, engine, name.to_string(), seq, kind);
    dialog.present();
}

pub fn apply_bytes(seq: u64, bytes: Vec<u8>) {
    TERMS.with(|m| {
        if let Some(term) = m.borrow().get(&seq) {
            term.feed(&bytes);
        }
    });
}

pub fn apply_term_done(seq: u64, result: Result<(), String>) {
    if let Err(err) = result {
        window::toast(&err);
        TERMS.with(|m| {
            if let Some(term) = m.borrow().get(&seq) {
                term.feed(format!("\r\n[{err}]\r\n").as_bytes());
            }
        });
    }
}

pub fn add_detail_actions(actions: &gtk::Box, host: HostId, engine: Engine, name: &str, _kind: Kind, state: RunState) {
    let running = state == RunState::Running;
    let logs = gtk::Button::with_label("Logs");
    let n = name.to_string();
    logs.connect_clicked(move |_| open_logs(host, engine, &n));
    actions.append(&logs);
    let term = gtk::Button::with_label("Terminal");
    let n = name.to_string();
    term.connect_clicked(move |_| open_term(host, engine, &n, TermKind::Exec, running));
    actions.append(&term);
    if engine == Engine::Incus {
        let console = gtk::Button::with_label("Console");
        let n = name.to_string();
        console.connect_clicked(move |_| open_term(host, engine, &n, TermKind::Console, running));
        actions.append(&console);
    }
}
