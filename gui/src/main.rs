mod calendar;
mod theme;

use std::collections::HashMap;

use caldav_core::{Db, Event, Reminder, Task};
use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use iced::widget::{button, checkbox, column, container, row, rule, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length, Task as IcedTask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Tasks,
    Events,
    Calendar,
}

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

// ponytail: duplicated from tui/src/main.rs's parse_remind_at rather than
// moved into core — core is out of scope for this pass. Move it there if a
// third consumer (e.g. daemon) ever needs the same parsing.
fn parse_datetime(input: &str) -> Option<i64> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('+') {
        let split = rest.len().checked_sub(1)?;
        let (num, unit) = rest.split_at(split);
        let n: i64 = num.parse().ok()?;
        let secs = match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86400,
            _ => return None,
        };
        return Some(caldav_core::db::now() + secs);
    }
    let ndt = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M").ok()?;
    Local.from_local_datetime(&ndt).single().map(|dt| dt.timestamp())
}

fn format_range(start_at: i64, end_at: i64) -> String {
    let fmt = "%a, %b %-d \u{b7} %H:%M";
    let start = Local.timestamp_opt(start_at, 0).single();
    let end = Local.timestamp_opt(end_at, 0).single();
    match (start, end) {
        (Some(s), Some(e)) => format!("{} \u{2013} {}", s.format(fmt), e.format("%H:%M")),
        _ => String::new(),
    }
}

struct App {
    db: Db,
    theme_mode: iced::theme::Mode,
    view: View,
    rows: Vec<TaskRow>,
    events: Vec<Event>,
    reminders: Vec<(Reminder, String)>,
    reminder_counts: HashMap<i64, i64>,
    input: String,
    adding_parent: Option<i64>,
    event_title: String,
    event_start: String,
    event_end: String,
    cal_scale: calendar::CalendarScale,
    cal_cursor: NaiveDate,
    status: String,
}

impl App {
    fn new() -> (Self, IcedTask<Message>) {
        let db = Db::open_default().expect("failed to open database");
        let mut app = App {
            db,
            theme_mode: iced::theme::Mode::Light,
            view: View::Tasks,
            rows: Vec::new(),
            events: Vec::new(),
            reminders: Vec::new(),
            reminder_counts: HashMap::new(),
            input: String::new(),
            adding_parent: None,
            event_title: String::new(),
            event_start: String::new(),
            event_end: String::new(),
            cal_scale: calendar::CalendarScale::Month,
            cal_cursor: Local::now().date_naive(),
            status: String::new(),
        };
        app.refresh();
        (app, iced::system::theme().map(Message::SystemThemeChanged))
    }

    fn refresh(&mut self) {
        let tasks = self.db.list_tasks().unwrap_or_default();
        self.reminder_counts = self.db.reminder_counts().unwrap_or_default();
        self.rows = build_tree(tasks);
        self.events = self.db.list_events().unwrap_or_default();
        self.reminders = self.db.list_all_reminders().unwrap_or_default();
    }

    fn theme(&self) -> iced::Theme {
        if self.theme_mode == iced::theme::Mode::Dark {
            theme::adwaita_dark()
        } else {
            theme::adwaita_light()
        }
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
    SwitchView(View),
    EventTitleChanged(String),
    EventStartChanged(String),
    EventEndChanged(String),
    SubmitEvent,
    DeleteEvent(i64),
    SystemThemeChanged(iced::theme::Mode),
    CalScaleChanged(calendar::CalendarScale),
    CalPrev,
    CalNext,
    CalToday,
    SelectDay(NaiveDate),
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
        Message::SwitchView(v) => app.view = v,
        Message::EventTitleChanged(s) => app.event_title = s,
        Message::EventStartChanged(s) => app.event_start = s,
        Message::EventEndChanged(s) => app.event_end = s,
        Message::SubmitEvent => {
            let title = app.event_title.trim().to_string();
            if title.is_empty() {
                app.status = "event needs a title".to_string();
                return;
            }
            match (parse_datetime(&app.event_start), parse_datetime(&app.event_end)) {
                (Some(start_at), Some(end_at)) if end_at <= start_at => {
                    app.status = "event end must be after start".to_string();
                }
                (Some(start_at), Some(end_at)) => {
                    let _ = app.db.create_event(&title, start_at, end_at);
                    app.event_title.clear();
                    app.event_start.clear();
                    app.event_end.clear();
                    app.status = "event added".to_string();
                    app.refresh();
                }
                _ => app.status = "couldn't parse event start/end time".to_string(),
            }
        }
        Message::DeleteEvent(id) => {
            let _ = app.db.delete_event(id);
            app.refresh();
        }
        Message::SystemThemeChanged(mode) => app.theme_mode = mode,
        Message::CalScaleChanged(scale) => app.cal_scale = scale,
        Message::CalPrev => app.cal_cursor = calendar::shift(app.cal_cursor, app.cal_scale, false),
        Message::CalNext => app.cal_cursor = calendar::shift(app.cal_cursor, app.cal_scale, true),
        Message::CalToday => app.cal_cursor = Local::now().date_naive(),
        Message::SelectDay(day) => {
            app.cal_cursor = day;
            app.cal_scale = calendar::CalendarScale::Day;
            let start = NaiveDateTime::new(day, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
            let end = NaiveDateTime::new(day, NaiveTime::from_hms_opt(10, 0, 0).unwrap());
            app.event_start = start.format("%Y-%m-%d %H:%M").to_string();
            app.event_end = end.format("%Y-%m-%d %H:%M").to_string();
        }
    }
}

fn header_bar(app: &App) -> Element<'_, Message> {
    let tab = |label: &'static str, target: View| {
        button(text(label).size(theme::SIZE_BODY))
            .style(theme::tab_button(app.view == target))
            .padding([theme::XS, theme::MD])
            .on_press(Message::SwitchView(target))
    };

    let bar = row![
        text("caldav").size(theme::SIZE_TITLE).font(theme::bold()),
        Space::new().width(Length::Fill),
        tab("Tasks", View::Tasks),
        tab("Events", View::Events),
        tab("Calendar", View::Calendar),
        Space::new().width(Length::Fill),
        button(text("Sync").size(theme::SIZE_BODY))
            .style(button::primary)
            .padding([theme::XS, theme::MD])
            .on_press(Message::SyncNow),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center);

    column![
        container(bar).padding([theme::SM, theme::MD]).width(Length::Fill),
        rule::horizontal(1).style(theme::divider),
    ]
    .into()
}

fn empty_state<'a>(title: &'a str, hint: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(title).size(theme::SIZE_TITLE),
            text(hint).size(theme::SIZE_META).style(theme::muted_text),
        ]
        .spacing(theme::XXS)
        .align_x(Alignment::Center),
    )
    .padding(theme::XL)
    .center_x(Length::Fill)
    .into()
}

fn status_line(app: &App) -> Element<'_, Message> {
    if app.status.is_empty() {
        Space::new().height(0.0).into()
    } else {
        text(app.status.clone()).size(theme::SIZE_CAPTION).style(theme::muted_text).into()
    }
}

fn task_row<'a>(app: &'a App, r: &'a TaskRow) -> Element<'a, Message> {
    let task_id = r.task.id;
    let has_reminder = app.reminder_counts.get(&task_id).copied().unwrap_or(0) > 0;

    let title = if r.task.done {
        text(r.task.title.clone()).size(theme::SIZE_BODY).style(theme::muted_text)
    } else {
        text(r.task.title.clone()).size(theme::SIZE_BODY)
    };

    let mut content = row![Space::new().width(r.depth as f32 * 24.0)]
        .spacing(theme::SM)
        .align_y(Alignment::Center)
        .padding([theme::XS, 0.0]);

    content = content.push(
        checkbox(r.task.done)
            .on_toggle(move |_| Message::ToggleDone(task_id))
            .style(theme::checkbox_style)
            .size(20)
            .spacing(theme::SM),
    );
    content = content.push(title);
    if has_reminder {
        content = content.push(text("reminder").size(theme::SIZE_CAPTION).style(theme::warning_text));
    }
    content = content.push(Space::new().width(Length::Fill));
    content = content.push(
        button(text("+").size(theme::SIZE_BODY))
            .style(theme::ghost_button)
            .on_press(Message::SetParent(task_id)),
    );
    content = content.push(
        button(text("x").size(theme::SIZE_META))
            .style(theme::danger_ghost_button)
            .on_press(Message::DeleteTask(task_id)),
    );

    content.into()
}

fn divided_list<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut col = column![];
    let last = items.len().saturating_sub(1);
    for (i, item) in items.into_iter().enumerate() {
        col = col.push(item);
        if i != last {
            col = col.push(rule::horizontal(1).style(theme::divider));
        }
    }
    scrollable(col).height(Length::Fill).into()
}

fn task_composer(app: &App) -> Element<'_, Message> {
    let breadcrumb: Element<'_, Message> = match app.adding_parent {
        Some(id) => {
            let label = app
                .rows
                .iter()
                .find(|r| r.task.id == id)
                .map(|r| r.task.title.clone())
                .unwrap_or_default();
            text(format!("Adding subtask to: {label}")).size(theme::SIZE_CAPTION).style(theme::muted_text).into()
        }
        None => Space::new().height(0.0).into(),
    };

    let mut input_row = row![
        text_input("New task title", &app.input)
            .on_input(Message::InputChanged)
            .on_submit(Message::Submit)
            .padding(theme::SM),
        button(text("Add")).style(button::primary).padding([theme::XS, theme::MD]).on_press(Message::Submit),
    ]
    .spacing(theme::SM);

    if app.adding_parent.is_some() {
        input_row = input_row.push(
            button(text("Cancel")).style(button::text).padding([theme::XS, theme::MD]).on_press(Message::ClearParent),
        );
    }

    container(column![breadcrumb, input_row, status_line(app)].spacing(theme::XXS))
        .padding(theme::MD)
        .style(theme::tinted_container)
        .width(Length::Fill)
        .into()
}

fn tasks_view(app: &App) -> Element<'_, Message> {
    let list = if app.rows.is_empty() {
        empty_state("No tasks yet", "Add one below to get started")
    } else {
        divided_list(app.rows.iter().map(|r| task_row(app, r)).collect())
    };

    column![task_composer(app), list].spacing(theme::MD).padding(theme::MD).into()
}

fn event_row(event: &Event) -> Element<'_, Message> {
    let event_id = event.id;
    row![
        column![
            text(event.title.clone()).size(theme::SIZE_BODY),
            text(format_range(event.start_at, event.end_at)).size(theme::SIZE_META).style(theme::muted_text),
        ]
        .spacing(theme::XXS),
        Space::new().width(Length::Fill),
        button(text("x").size(theme::SIZE_META))
            .style(theme::danger_ghost_button)
            .on_press(Message::DeleteEvent(event_id)),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center)
    .padding([theme::XS, 0.0])
    .into()
}

fn event_composer(app: &App) -> Element<'_, Message> {
    let input_row = row![
        text_input("Event title", &app.event_title)
            .on_input(Message::EventTitleChanged)
            .padding(theme::SM)
            .width(Length::Fill),
        text_input("Start: YYYY-MM-DD HH:MM", &app.event_start)
            .on_input(Message::EventStartChanged)
            .padding(theme::SM)
            .width(Length::Fixed(190.0)),
        text_input("End: YYYY-MM-DD HH:MM", &app.event_end)
            .on_input(Message::EventEndChanged)
            .on_submit(Message::SubmitEvent)
            .padding(theme::SM)
            .width(Length::Fixed(190.0)),
        button(text("Add")).style(button::primary).padding([theme::XS, theme::MD]).on_press(Message::SubmitEvent),
    ]
    .spacing(theme::SM);

    container(column![input_row, status_line(app)].spacing(theme::XXS))
        .padding(theme::MD)
        .style(theme::tinted_container)
        .width(Length::Fill)
        .into()
}

fn events_view(app: &App) -> Element<'_, Message> {
    let list = if app.events.is_empty() {
        empty_state("No events yet", "Add one below \u{2014} synced events will appear here too")
    } else {
        divided_list(app.events.iter().map(event_row).collect())
    };

    column![event_composer(app), list].spacing(theme::MD).padding(theme::MD).into()
}

fn view(app: &App) -> Element<'_, Message> {
    let body = match app.view {
        View::Tasks => tasks_view(app),
        View::Events => events_view(app),
        View::Calendar => calendar::view(app),
    };

    column![header_bar(app), body].into()
}

pub fn main() -> iced::Result {
    iced::application(App::new, update, view)
        .title("caldav")
        .theme(|app: &App| app.theme())
        .subscription(|_| iced::system::theme_changes().map(Message::SystemThemeChanged))
        .run()
}
