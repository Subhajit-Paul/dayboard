use std::collections::HashMap;

use caldav_core::{Db, Task};
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};

struct TaskRow {
    task: Task,
    depth: usize,
}

fn build_tree(tasks: Vec<Task>) -> Vec<TaskRow> {
    let mut children: HashMap<i64, Vec<Task>> = HashMap::new();
    let mut roots: Vec<Task> = Vec::new();
    for t in tasks {
        match t.parent_id {
            Some(pid) => children.entry(pid).or_default().push(t),
            None => roots.push(t),
        }
    }
    let mut rows = Vec::new();
    for root in roots {
        let kids = children.remove(&root.id).unwrap_or_default();
        rows.push(TaskRow { task: root, depth: 0 });
        for k in kids {
            rows.push(TaskRow { task: k, depth: 1 });
        }
    }
    rows
}

struct App {
    db: Db,
    rows: Vec<TaskRow>,
    reminder_counts: HashMap<i64, i64>,
    input: String,
    adding_parent: Option<i64>,
    status: String,
}

impl App {
    fn new() -> Self {
        let db = Db::open_default().expect("failed to open database");
        let mut app = App {
            db,
            rows: Vec::new(),
            reminder_counts: HashMap::new(),
            input: String::new(),
            adding_parent: None,
            status: String::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let tasks = self.db.list_tasks().unwrap_or_default();
        self.reminder_counts = self.db.reminder_counts().unwrap_or_default();
        self.rows = build_tree(tasks);
    }
}

#[derive(Debug, Clone)]
enum Message {
    InputChanged(String),
    Submit,
    SetParent(i64),
    ClearParent,
    ToggleDone(i64),
    DeleteTask(i64),
    SyncNow,
}

fn update(app: &mut App, message: Message) {
    match message {
        Message::InputChanged(s) => app.input = s,
        Message::Submit => {
            let title = app.input.trim().to_string();
            if !title.is_empty() {
                let _ = app.db.create_task(&title, app.adding_parent);
                app.input.clear();
                app.adding_parent = None;
                app.refresh();
            }
        }
        Message::SetParent(id) => app.adding_parent = Some(id),
        Message::ClearParent => app.adding_parent = None,
        Message::ToggleDone(id) => {
            let _ = app.db.toggle_done(id);
            app.refresh();
        }
        Message::DeleteTask(id) => {
            let _ = app.db.delete_task(id);
            app.refresh();
        }
        Message::SyncNow => {
            if !caldav_core::auth::is_authenticated() {
                app.status = "not connected \u{2014} run `caldavd --auth` in a terminal first".to_string();
            } else {
                match caldav_core::sync::run(&app.db) {
                    Ok(()) => {
                        app.status = "sync complete".to_string();
                        app.refresh();
                    }
                    Err(e) => app.status = format!("sync failed: {e}"),
                }
            }
        }
    }
}

fn view(app: &App) -> Element<'_, Message> {
    let mut list = column![].spacing(6);
    for r in &app.rows {
        let indent = "  ".repeat(r.depth);
        let bell = if app.reminder_counts.get(&r.task.id).copied().unwrap_or(0) > 0 {
            " \u{23F0}"
        } else {
            ""
        };
        let label = format!("{indent}{}{bell}", r.task.title);
        let task_id = r.task.id;

        let item = row![
            checkbox(r.task.done).label(label).on_toggle(move |_| Message::ToggleDone(task_id)),
            button(text("+ subtask")).on_press(Message::SetParent(task_id)),
            button(text("delete")).on_press(Message::DeleteTask(task_id)),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        list = list.push(item);
    }

    let parent_label = match app.adding_parent {
        Some(id) => app
            .rows
            .iter()
            .find(|r| r.task.id == id)
            .map(|r| format!("adding subtask to: {}", r.task.title))
            .unwrap_or_default(),
        None => String::new(),
    };

    let input_row = row![
        text_input("New task title", &app.input)
            .on_input(Message::InputChanged)
            .on_submit(Message::Submit),
        button(text("Add")).on_press(Message::Submit),
        button(text("clear parent")).on_press(Message::ClearParent),
    ]
    .spacing(8);

    let content = column![
        text("caldav").size(24),
        input_row,
        text(parent_label),
        scrollable(list).height(Length::Fill),
        row![
            button(text("Sync Now")).on_press(Message::SyncNow),
            text(app.status.clone()),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .padding(16);

    container(content).into()
}

pub fn main() -> iced::Result {
    iced::application(App::new, update, view).title("caldav").run()
}
