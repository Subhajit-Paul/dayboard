use std::collections::HashMap;
use std::io;

use caldav_core::{Db, Event as CalEvent, Reminder, Task};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::calendar::{CalendarEventStore, Monthly};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

// Adwaita blue, kept consistent with the GUI's accent token.
const ACCENT: Color = Color::Rgb(0x35, 0x84, 0xe4);
const WARNING: Color = Color::Rgb(0xe5, 0xa5, 0x0a);
const DANGER: Color = Color::Rgb(0xe0, 0x1b, 0x24);

#[derive(PartialEq, Eq, Clone, Copy)]
enum Pane {
    Tasks,
    Events,
    Calendar,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CalScale {
    Day,
    Week,
    Month,
}

enum Purpose {
    AddTask,
    AddSubtask(i64),
    EditTask(i64),
    AddReminder(i64),
    AddEvent { preset_date: Option<NaiveDate> },
}

enum Mode {
    Normal,
    Input { purpose: Purpose, buffer: String },
    ConfirmDelete { task_id: i64, has_children: bool },
}

struct Row {
    task: Task,
    depth: usize,
}

struct App {
    db: Db,
    pane: Pane,
    rows: Vec<Row>,
    events: Vec<CalEvent>,
    reminders: Vec<(Reminder, String)>,
    reminder_counts: HashMap<i64, i64>,
    selected: usize,
    cal_scale: CalScale,
    cal_cursor: NaiveDate,
    mode: Mode,
    status: String,
}

const HELP: &str = "a add  s subtask  e edit  d delete  space toggle  r reminder  v event  g sync  \
Tab pane  1/2/3 scale  h/l nav  t today  q quit";

impl App {
    fn new(db: Db) -> Self {
        let mut app = App {
            db,
            pane: Pane::Tasks,
            rows: Vec::new(),
            events: Vec::new(),
            reminders: Vec::new(),
            reminder_counts: HashMap::new(),
            selected: 0,
            cal_scale: CalScale::Month,
            cal_cursor: Local::now().date_naive(),
            mode: Mode::Normal,
            status: String::new(),
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        let tasks = self.db.list_tasks().unwrap_or_default();
        self.reminder_counts = self.db.reminder_counts().unwrap_or_default();
        self.rows = build_tree(tasks);
        self.events = self.db.list_events().unwrap_or_default();
        self.reminders = self.db.list_all_reminders().unwrap_or_default();
        if self.selected >= self.rows.len() && !self.rows.is_empty() {
            self.selected = self.rows.len() - 1;
        }
    }

    fn selected_task_id(&self) -> Option<i64> {
        self.rows.get(self.selected).map(|r| r.task.id)
    }
}

fn build_tree(tasks: Vec<Task>) -> Vec<Row> {
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
        rows.push(Row { task: root, depth: 0 });
        for k in kids {
            rows.push(Row { task: k, depth: 1 });
        }
    }
    rows
}

fn parse_remind_at(input: &str) -> Option<i64> {
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

fn to_naive_date(ts: i64) -> NaiveDate {
    Local.timestamp_opt(ts, 0).single().expect("valid unix timestamp").date_naive()
}

// ponytail: duplicated from gui/src/calendar.rs's week_start rather than
// moved into core — same reasoning as parse_remind_at above.
fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// Bridges our chrono dates to the `time` crate's `Date`, which is what
/// ratatui's built-in `Monthly` calendar widget requires.
fn to_time_date(d: NaiveDate) -> Option<time::Date> {
    time::Date::from_calendar_date(d.year(), time::Month::try_from(d.month() as u8).ok()?, d.day() as u8).ok()
}

fn main() -> io::Result<()> {
    let db = Db::open_default().expect("failed to open database");
    let mut app = App::new(db);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if handle_key(app, key.code) {
                return Ok(());
            }
        }
    }
}

/// Returns true when the app should quit.
fn handle_key(app: &mut App, code: KeyCode) -> bool {
    let mode = std::mem::replace(&mut app.mode, Mode::Normal);
    match mode {
        Mode::Normal => return handle_normal_key(app, code),
        Mode::Input { purpose, mut buffer } => match code {
            KeyCode::Esc => app.mode = Mode::Normal,
            KeyCode::Enter => submit_input(app, purpose, buffer),
            KeyCode::Backspace => {
                buffer.pop();
                app.mode = Mode::Input { purpose, buffer };
            }
            KeyCode::Char(c) => {
                buffer.push(c);
                app.mode = Mode::Input { purpose, buffer };
            }
            _ => app.mode = Mode::Input { purpose, buffer },
        },
        Mode::ConfirmDelete { task_id, .. } => {
            if code == KeyCode::Char('y') {
                let _ = app.db.delete_task(task_id);
                app.refresh();
            }
            app.mode = Mode::Normal;
        }
    }
    false
}

fn handle_normal_key(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Tab => {
            app.pane = match app.pane {
                Pane::Tasks => Pane::Events,
                Pane::Events => Pane::Calendar,
                Pane::Calendar => Pane::Tasks,
            };
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.pane == Pane::Tasks && app.selected > 0 {
                app.selected -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.pane == Pane::Tasks && app.selected + 1 < app.rows.len() {
                app.selected += 1;
            }
        }
        KeyCode::Char('1') if app.pane == Pane::Calendar => app.cal_scale = CalScale::Day,
        KeyCode::Char('2') if app.pane == Pane::Calendar => app.cal_scale = CalScale::Week,
        KeyCode::Char('3') if app.pane == Pane::Calendar => app.cal_scale = CalScale::Month,
        KeyCode::Char('h') | KeyCode::Left if app.pane == Pane::Calendar => {
            app.cal_cursor -= Duration::days(1);
        }
        KeyCode::Char('l') | KeyCode::Right if app.pane == Pane::Calendar => {
            app.cal_cursor += Duration::days(1);
        }
        KeyCode::Char('t') if app.pane == Pane::Calendar => {
            app.cal_cursor = Local::now().date_naive();
        }
        KeyCode::Char('a') => {
            app.mode = Mode::Input {
                purpose: Purpose::AddTask,
                buffer: String::new(),
            };
        }
        KeyCode::Char('s') => match app.selected_task_id() {
            Some(id) => {
                app.mode = Mode::Input {
                    purpose: Purpose::AddSubtask(id),
                    buffer: String::new(),
                }
            }
            None => app.status = "select a task first".into(),
        },
        KeyCode::Char('e') => {
            if let Some(row) = app.rows.get(app.selected) {
                app.mode = Mode::Input {
                    purpose: Purpose::EditTask(row.task.id),
                    buffer: row.task.title.clone(),
                };
            }
        }
        KeyCode::Char('d') => {
            if let Some(id) = app.selected_task_id() {
                let has_children = app.db.has_children(id).unwrap_or(false);
                app.mode = Mode::ConfirmDelete {
                    task_id: id,
                    has_children,
                };
            }
        }
        KeyCode::Char(' ') | KeyCode::Enter if app.pane == Pane::Tasks => {
            if let Some(id) = app.selected_task_id() {
                let _ = app.db.toggle_done(id);
                app.refresh();
            }
        }
        KeyCode::Enter if app.pane == Pane::Calendar => {
            app.mode = Mode::Input {
                purpose: Purpose::AddEvent { preset_date: Some(app.cal_cursor) },
                buffer: String::new(),
            };
            app.status = format!(
                "event title for {} (09:00\u{2013}10:00), Enter to add",
                app.cal_cursor.format("%Y-%m-%d")
            );
        }
        KeyCode::Char('r') => match app.selected_task_id() {
            Some(id) => {
                app.mode = Mode::Input {
                    purpose: Purpose::AddReminder(id),
                    buffer: String::new(),
                };
                app.status = "reminder time: YYYY-MM-DD HH:MM or +30s / +5m / +1h".into();
            }
            None => app.status = "select a task first".into(),
        },
        KeyCode::Char('v') => {
            app.mode = Mode::Input {
                purpose: Purpose::AddEvent { preset_date: None },
                buffer: String::new(),
            };
            app.status = "event: Title | YYYY-MM-DD HH:MM | YYYY-MM-DD HH:MM".into();
        }
        KeyCode::Char('g') => {
            if !caldav_core::auth::is_authenticated() {
                app.status = "not connected — run `caldavd --auth` in a terminal first".into();
            } else {
                app.status = "syncing...".into();
                match caldav_core::sync::run(&app.db) {
                    Ok(()) => {
                        app.status = "sync complete".into();
                        app.refresh();
                    }
                    Err(e) => app.status = format!("sync failed: {e}"),
                }
            }
        }
        _ => {}
    }
    false
}

fn submit_input(app: &mut App, purpose: Purpose, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        app.mode = Mode::Normal;
        return;
    }
    match purpose {
        Purpose::AddTask => {
            let _ = app.db.create_task(&text, None);
        }
        Purpose::AddSubtask(parent_id) => {
            let _ = app.db.create_task(&text, Some(parent_id));
        }
        Purpose::EditTask(id) => {
            let _ = app.db.update_task_title(id, &text);
        }
        Purpose::AddReminder(task_id) => match parse_remind_at(&text) {
            Some(ts) => {
                let _ = app.db.create_reminder(task_id, ts);
                app.status = "reminder set".into();
            }
            None => app.status = format!("couldn't parse time: {text}"),
        },
        Purpose::AddEvent { preset_date: None } => {
            let parts: Vec<&str> = text.splitn(3, '|').map(str::trim).collect();
            match parts.as_slice() {
                [title, start, end] => match (parse_remind_at(start), parse_remind_at(end)) {
                    (Some(start_at), Some(end_at)) => {
                        let _ = app.db.create_event(title, start_at, end_at);
                        app.status = "event added".into();
                    }
                    _ => app.status = "couldn't parse event start/end time".into(),
                },
                _ => app.status = "format: Title | YYYY-MM-DD HH:MM | YYYY-MM-DD HH:MM".into(),
            }
        }
        Purpose::AddEvent { preset_date: Some(date) } => {
            let start = NaiveDateTime::new(date, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
            let end = NaiveDateTime::new(date, NaiveTime::from_hms_opt(10, 0, 0).unwrap());
            match (Local.from_local_datetime(&start).single(), Local.from_local_datetime(&end).single()) {
                (Some(s), Some(e)) => {
                    let _ = app.db.create_event(&text, s.timestamp(), e.timestamp());
                    app.status = "event added".into();
                }
                _ => app.status = "couldn't resolve local time for that day".into(),
            }
        }
    }
    app.mode = Mode::Normal;
    app.refresh();
}

fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3), Constraint::Length(3)])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled(" caldav ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw(HELP),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, chunks[0]);

    let border_style = Style::default().fg(Color::DarkGray);
    let title_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);

    match app.pane {
        Pane::Tasks => {
            let items: Vec<ListItem> = app
                .rows
                .iter()
                .map(|row| {
                    let indent = if row.depth > 0 { "  \u{21b3} " } else { "" };
                    let check = if row.task.done { "[x] " } else { "[ ] " };
                    let style = if row.task.done {
                        Style::default().add_modifier(Modifier::CROSSED_OUT).fg(Color::DarkGray)
                    } else {
                        Style::default()
                    };
                    let mut spans = vec![Span::styled(format!("{indent}{check}{}", row.task.title), style)];
                    if app.reminder_counts.get(&row.task.id).copied().unwrap_or(0) > 0 {
                        spans.push(Span::styled(" (reminder)", Style::default().fg(WARNING)));
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect();

            let mut state = ListState::default();
            if !app.rows.is_empty() {
                state.select(Some(app.selected));
            }

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(Span::styled(" Tasks ", title_style)),
                )
                .highlight_style(Style::default().bg(ACCENT).fg(Color::White));

            frame.render_stateful_widget(list, chunks[1], &mut state);
        }
        Pane::Events => {
            let items: Vec<ListItem> = app
                .events
                .iter()
                .map(|event| {
                    let range = match (
                        Local.timestamp_opt(event.start_at, 0).single(),
                        Local.timestamp_opt(event.end_at, 0).single(),
                    ) {
                        (Some(s), Some(e)) => {
                            format!("{} \u{2013} {}", s.format("%a %b %-d %H:%M"), e.format("%H:%M"))
                        }
                        _ => String::new(),
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{}  ", event.title)),
                        Span::styled(range, Style::default().fg(Color::DarkGray)),
                    ]))
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(Span::styled(" Events ", title_style)),
            );
            frame.render_widget(list, chunks[1]);
        }
        Pane::Calendar => draw_calendar(frame, chunks[1], app, border_style, title_style),
    }

    let bottom_text = match &app.mode {
        Mode::Normal => app.status.clone(),
        Mode::Input { buffer, .. } => format!("> {buffer}"),
        Mode::ConfirmDelete { has_children, .. } => {
            if *has_children {
                "delete task and its subtasks? y/n".to_string()
            } else {
                "delete task? y/n".to_string()
            }
        }
    };
    let bottom_border_color = match &app.mode {
        Mode::Normal => Color::DarkGray,
        Mode::Input { .. } => ACCENT,
        Mode::ConfirmDelete { .. } => DANGER,
    };
    let bottom = Paragraph::new(bottom_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(bottom_border_color)),
    );
    frame.render_widget(bottom, chunks[2]);
}

/// Events and reminders due on `day`, as ready-to-render list items — the
/// shared building block for both the Day view and each day-section of the
/// Week view.
fn calendar_items_for_day(app: &App, day: NaiveDate) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    for event in app.events.iter().filter(|e| to_naive_date(e.start_at) == day) {
        let range = match (
            Local.timestamp_opt(event.start_at, 0).single(),
            Local.timestamp_opt(event.end_at, 0).single(),
        ) {
            (Some(s), Some(e)) => format!("{} \u{2013} {}", s.format("%H:%M"), e.format("%H:%M")),
            _ => String::new(),
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{}  ", event.title)),
            Span::styled(range, Style::default().fg(Color::DarkGray)),
        ])));
    }
    for (reminder, title) in app.reminders.iter().filter(|(r, _)| to_naive_date(r.remind_at) == day) {
        let time = Local
            .timestamp_opt(reminder.remind_at, 0)
            .single()
            .map(|dt| dt.format("%H:%M").to_string())
            .unwrap_or_default();
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("{title}  ")),
            Span::styled(format!("{time} (reminder)"), Style::default().fg(WARNING)),
        ])));
    }
    items
}

fn week_items(app: &App) -> Vec<ListItem<'static>> {
    let start = week_start(app.cal_cursor);
    let mut items = Vec::new();
    for i in 0..7 {
        let day = start + Duration::days(i);
        let header_style = if day == app.cal_cursor {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        };
        items.push(ListItem::new(Span::styled(day.format("%a %b %-d").to_string(), header_style)));

        let day_items = calendar_items_for_day(app, day);
        if day_items.is_empty() {
            items.push(ListItem::new(Span::styled("  (nothing)", Style::default().fg(Color::DarkGray))));
        } else {
            items.extend(day_items);
        }
    }
    items
}

/// Builds the day -> style map for the Month view. ponytail: a day with
/// both an event and a reminder just shows the event's style (event styled
/// last, so it wins) rather than a merged style — `CalendarEventStore::add`
/// is last-write-wins by design, and combining styles would mean reaching
/// into its internal map type directly. Upgrade if that distinction turns
/// out to matter in practice.
fn calendar_event_store(app: &App) -> CalendarEventStore {
    let mut store = CalendarEventStore(Default::default());
    for (reminder, _) in &app.reminders {
        if let Some(td) = to_time_date(to_naive_date(reminder.remind_at)) {
            store.add(td, Style::default().fg(WARNING).add_modifier(Modifier::BOLD));
        }
    }
    for event in &app.events {
        if let Some(td) = to_time_date(to_naive_date(event.start_at)) {
            store.add(td, Style::default().bg(ACCENT).fg(Color::White));
        }
    }
    if let Some(td) = to_time_date(app.cal_cursor) {
        store.add(td, Style::default().add_modifier(Modifier::REVERSED));
    }
    store
}

fn draw_calendar(frame: &mut Frame, area: Rect, app: &App, border_style: Style, title_style: Style) {
    match app.cal_scale {
        CalScale::Month => {
            let Some(display_date) = to_time_date(app.cal_cursor) else { return };
            let store = calendar_event_store(app);
            let monthly = Monthly::new(display_date, store)
                .show_month_header(title_style)
                .show_weekdays_header(border_style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(border_style)
                        .title(Span::styled(" Calendar ", title_style)),
                );
            frame.render_widget(monthly, area);
        }
        CalScale::Week => {
            let list = List::new(week_items(app)).block(
                Block::default().borders(Borders::ALL).border_style(border_style).title(Span::styled(
                    format!(" Week of {} ", week_start(app.cal_cursor).format("%b %-d")),
                    title_style,
                )),
            );
            frame.render_widget(list, area);
        }
        CalScale::Day => {
            let list = List::new(calendar_items_for_day(app, app.cal_cursor)).block(
                Block::default().borders(Borders::ALL).border_style(border_style).title(Span::styled(
                    format!(" {} ", app.cal_cursor.format("%A, %b %-d")),
                    title_style,
                )),
            );
            frame.render_widget(list, area);
        }
    }
}
