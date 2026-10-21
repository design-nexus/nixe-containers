use crate::models::{Action, HostFilter, HostId, Snapshot, Stack};
use crate::services::compose;
use crate::widgets::{self, Col, OpsTable, Page};
use crate::{window, worker};
use gtk::gio;
use gtk::prelude::*;
use sourceview5::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct List {
    table: OpsTable,
    count: gtk::Label,
}

thread_local! {
    static UI: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
}

fn cols() -> [Col; 5] {
    [
        widgets::col("name", "Project", true, false, 0),
        widgets::col("host", "Host", false, false, 88),
        widgets::col("services", "Services", false, true, 88),
        widgets::col("status", "Status", false, false, 96),
        widgets::col("actions", "", false, true, 168),
    ]
}

pub fn build(page: &Page) {
    page.fill();
    let (bar, count, actions) = widgets::page_header("Stacks");
    let create = gtk::Button::with_label("New stack");
    create.add_css_class("suggested-action");
    create.connect_clicked(|_| open_editor(None));
    actions.append(&create);
    page.body.append(&bar);
    let table = OpsTable::new(
        &cols(),
        "view-list-symbolic",
        "No Compose stacks",
        "Projects show up from Docker Compose labels, or create one with a compose.yaml.",
    );
    page.body.append(&table.root);
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(List { table, count })));
}

pub fn refresh(snapshot: &Snapshot, host: HostFilter, query: &str) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    ui.table.clear();
    let q = query.trim().to_ascii_lowercase();
    let mut shown = 0;
    for stack in snapshot.stacks(host) {
        if !q.is_empty() && !stack.name.to_ascii_lowercase().contains(&q) {
            continue;
        }
        ui.table.push(stack_cells(&stack));
        shown += 1;
    }
    ui.table.set_count(shown);
    widgets::set_count(&ui.count, shown, "stack");
}

fn stack_cells(stack: &Stack) -> Vec<gtk::Widget> {
    let status = if stack.running == stack.total && stack.total > 0 {
        widgets::status_pill("Running", "ok").upcast()
    } else if stack.running > 0 {
        widgets::status_pill("Partial", "").upcast()
    } else {
        widgets::status_pill("Stopped", "").upcast()
    };
    let actions = widgets::actions_cell();
    let host = stack.host;
    let names = stack.names.clone();
    if stack.running < stack.total {
        let up = widgets::icon_button("media-playback-start-symbolic", "Up");
        let s = stack.clone();
        up.connect_clicked(move |_| run_stack(&s, Action::Start));
        actions.append(&up);
    }
    if stack.running > 0 {
        let down = widgets::icon_button("media-playback-stop-symbolic", "Down");
        let s = stack.clone();
        down.connect_clicked(move |_| run_stack(&s, Action::Stop));
        actions.append(&down);
        let restart = widgets::icon_button("view-refresh-symbolic", "Restart");
        let s = stack.clone();
        restart.connect_clicked(move |_| run_stack(&s, Action::Restart));
        actions.append(&restart);
    }
    let logs = widgets::icon_button("utilities-system-monitor-symbolic", "Logs");
    let n = names.clone();
    let project = stack.name.clone();
    logs.connect_clicked(move |_| {
        crate::sections::session::open_stack_logs(host, crate::models::Engine::Docker, &project, n.clone())
    });
    actions.append(&logs);
    let edit = widgets::icon_button("document-edit-symbolic", "Edit YAML");
    let s = stack.clone();
    edit.connect_clicked(move |_| open_editor(Some(s.clone())));
    actions.append(&edit);
    vec![
        widgets::td(&stack.name),
        widgets::td_dim(&window::host_name(stack.host)),
        widgets::td_dim(&format!("{}/{}", stack.running, stack.total)),
        status,
        actions.upcast(),
    ]
}

fn run_stack(stack: &Stack, action: Action) {
    window::toast(&format!("{} {}…", action.gerund(), stack.name));
    if let Some((dir, file)) = compose_paths(stack)
        && stack.host == HostId::Local
    {
        worker::compose(stack.host, stack.name.clone(), dir, file, action);
        return;
    }
    worker::stack_op(stack.host, stack.names.clone(), action);
}

fn compose_paths(stack: &Stack) -> Option<(String, String)> {
    let dir = stack.project_dir.clone().filter(|s| !s.is_empty())?;
    let file = stack.compose_file.clone().unwrap_or_else(|| default_compose_path(Path::new(&dir)).display().to_string());
    Some((dir, file))
}

fn default_compose_path(dir: &Path) -> PathBuf {
    for name in ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"] {
        let p = dir.join(name);
        if p.exists() {
            return p;
        }
    }
    dir.join("compose.yaml")
}

fn open_editor(existing: Option<Stack>) {
    let title = existing.as_ref().map(|s| format!("Edit {}", s.name)).unwrap_or_else(|| "New stack".into());
    let (dialog, card) = widgets::dialog(&title, 780);
    dialog.set_default_height(640);

    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Project name"));
    name.set_hexpand(true);
    if let Some(s) = &existing {
        name.set_text(&s.name);
    }
    card.append(&widgets::row("Name", "Compose project name.", Some(name.upcast_ref())));

    let dir = gtk::Entry::new();
    dir.set_placeholder_text(Some("/path/to/project"));
    dir.set_hexpand(true);
    if let Some(s) = &existing
        && let Some(d) = &s.project_dir
    {
        dir.set_text(d);
    }
    let browse = gtk::Button::with_label("Browse");
    let dir_row = widgets::hbox(8);
    dir_row.append(&dir);
    dir_row.append(&browse);
    card.append(&widgets::row("Directory", "YAML is saved here. Deploy runs on this computer.", Some(dir_row.upcast_ref())));

    let d_for_browse = dir.clone();
    let parent = window::window();
    browse.connect_clicked(move |_| {
        let chooser = gtk::FileDialog::new();
        chooser.set_title("Project directory");
        let dest = d_for_browse.clone();
        chooser.select_folder(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                dest.set_text(&path.display().to_string());
            }
        });
    });

    let buffer = sourceview5::Buffer::new(None::<&gtk::TextTagTable>);
    buffer.set_language(sourceview5::LanguageManager::default().language("yaml").as_ref());
    widgets::apply_source_scheme(&buffer);
    let yaml_text = existing.as_ref().and_then(load_yaml).unwrap_or_else(|| compose::TEMPLATE.to_string());
    buffer.set_text(&yaml_text);
    let view = sourceview5::View::with_buffer(&buffer);
    view.set_monospace(true);
    view.set_show_line_numbers(true);
    view.set_highlight_current_line(true);
    view.set_auto_indent(true);
    view.set_tab_width(2);
    view.set_insert_spaces_instead_of_tabs(true);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.add_css_class("compose-editor");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .min_content_height(320)
        .child(&view)
        .build();
    card.append(&scroll);

    let error = widgets::label("", "danger");
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.set_visible(false);
    card.append(&error);

    let local_only = existing.as_ref().is_some_and(|s| s.host != HostId::Local);
    if local_only {
        let note = widgets::label("This stack is on the remote host. Save writes a local file; Deploy stays on this computer.", "dim");
        note.set_wrap(true);
        note.set_xalign(0.0);
        card.append(&note);
    }

    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let save = gtk::Button::with_label("Save");
    let deploy = gtk::Button::with_label("Deploy");
    deploy.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&save);
    buttons.append(&deploy);
    card.append(&buttons);

    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());

    let save_fn = {
        let name = name.clone();
        let dir = dir.clone();
        let buffer = buffer.clone();
        let error = error.clone();
        let existing = existing.clone();
        move || match write_compose(&name.text(), &dir.text(), &buffer, existing.as_ref()) {
            Ok((project, dir, file)) => {
                error.set_visible(false);
                window::toast(&format!("Saved {file}"));
                Ok((project, dir, file))
            }
            Err(err) => {
                error.set_text(&err);
                error.set_visible(true);
                Err(err)
            }
        }
    };
    let save_click = save_fn.clone();
    save.connect_clicked(move |_| {
        let _ = save_click();
    });
    let d = dialog.clone();
    deploy.connect_clicked(move |_| {
        if let Ok((project, dir, file)) = save_fn() {
            window::toast(&format!("Deploying {project}…"));
            worker::compose(HostId::Local, project, dir, file, Action::Start);
            d.close();
        }
    });
    dialog.present();
}

fn load_yaml(stack: &Stack) -> Option<String> {
    let (_, file) = compose_paths(stack)?;
    std::fs::read_to_string(file).ok()
}

fn write_compose(
    name: &str,
    dir: &str,
    buffer: &sourceview5::Buffer,
    existing: Option<&Stack>,
) -> Result<(String, String, String), String> {
    let project = name.trim();
    if project.is_empty() {
        return Err("A project name is required.".into());
    }
    let dir = dir.trim();
    if dir.is_empty() {
        return Err("A project directory is required.".into());
    }
    let yaml = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
    compose::validate_yaml(&yaml)?;
    let path = existing
        .and_then(compose_paths)
        .map(|(_, file)| PathBuf::from(file))
        .unwrap_or_else(|| default_compose_path(Path::new(dir)));
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {dir}: {e}"))?;
    crate::cmd::atomic_write(&path, &yaml).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok((project.to_string(), dir.to_string(), path.display().to_string()))
}
