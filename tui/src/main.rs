use std::collections::HashMap;
use std::io;

use caldav_core::{Db, Event as CalEvent, Reminder, Task};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
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
    nerd: bool,
}

/// Text markers, swapped for Nerd Font glyphs when the user enables them (they
/// only render on a terminal using a Nerd Font, so this is opt-in). Toggle at
/// runtime with `f`, or default it on with `CALDAV_TUI_NERD=1`.
struct Glyphs {
    check_on: &'static str,
    check_off: &'static str,
    bell: &'static str,
    event: &'static str,
    tab_tasks: &'static str,
    tab_events: &'static str,
    tab_calendar: &'static str,
}

impl Glyphs {
    fn new(nerd: bool) -> Self {
        if nerd {
            Glyphs {
                check_on: "\u{f14a} ",   // nf-fa-check_square
                check_off: "\u{f096} ",  // nf-fa-square_o
                bell: "\u{f0f3} ",       // nf-fa-bell
                event: "\u{f017} ",      // nf-fa-clock_o
                tab_tasks: "\u{f0ae} ",  // nf-fa-tasks
                tab_events: "\u{f017} ", // nf-fa-clock_o
                tab_calendar: "\u{f073} ", // nf-fa-calendar
            }
        } else {
            Glyphs {
                check_on: "[x] ",
                check_off: "[ ] ",
                bell: "",
                event: "",
                tab_tasks: "",
                tab_events: "",
                tab_calendar: "",
            }
        }
    }
}

/// Context-sensitive key hints for the focused pane, shown dim in the footer
/// when idle — far friendlier than one wall-of-text help line.
fn contextual_help(pane: Pane) -> &'static str {
    match pane {
        Pane::Tasks => "a add  s subtask  e edit  d delete  space done  r reminder  g sync  f fonts  Tab pane  q quit",
        Pane::Events => "v add event  g sync  f fonts  Tab pane  q quit",
        Pane::Calendar => "1/2/3 day/week/month  h/l move  t today  Enter add  f fonts  Tab pane  q quit",
    }
}

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
            nerd: matches!(std::env::var("CALDAV_TUI_NERD").as_deref(), Ok("1") | Ok("true")),
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

/// 42 Monday-first days (6 weeks) covering the month containing `cursor`.
/// ponytail: mirrors gui/src/calendar.rs's `month_grid`; kept per-frontend.
fn month_days(cursor: NaiveDate) -> Vec<NaiveDate> {
    let first = cursor.with_day(1).expect("day 1 is always valid");
    let start = week_start(first);
    (0..42).map(|i| start + Duration::days(i)).collect()
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
        KeyCode::Char('f') => {
            app.nerd = !app.nerd;
            app.status = if app.nerd {
                "Nerd Font glyphs on (needs a Nerd Font in your terminal)".into()
            } else {
                "Nerd Font glyphs off".into()
            };
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

/// Focused-pane block: an accent border + accent title, so the active pane
/// reads as focused (only one pane is ever shown at a time).
fn pane_block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
}

/// Renders a centered, dim empty-state message inside `block`. The TUI's
/// stand-in for the GUI's illustrated empty states.
fn render_empty(frame: &mut Frame, area: Rect, block: Block<'static>, title: &str, hint: &str) {
    // nudge the message toward vertical center without a full layout pass.
    let pad = (area.height.saturating_sub(4) / 2) as usize;
    let mut lines: Vec<Line> = vec![Line::from(""); pad];
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(hint.to_string(), Style::default().fg(Color::DarkGray))));
    let para = Paragraph::new(Text::from(lines)).alignment(Alignment::Center).block(block);
    frame.render_widget(para, area);
}

/// Events grouped under bold accent day headers, mirroring the GUI's
/// date-grouped event cards. Relies on `list_events` being start-ordered.
fn events_items(app: &App) -> Vec<ListItem<'static>> {
    let g = Glyphs::new(app.nerd);
    let mut items = Vec::new();
    let mut last_date: Option<NaiveDate> = None;
    for event in &app.events {
        let day = to_naive_date(event.start_at);
        if last_date != Some(day) {
            if last_date.is_some() {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Span::styled(
                day.format("%A, %b %-d").to_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
            last_date = Some(day);
        }
        let range = match (
            Local.timestamp_opt(event.start_at, 0).single(),
            Local.timestamp_opt(event.end_at, 0).single(),
        ) {
            (Some(s), Some(e)) => format!("{} \u{2013} {}", s.format("%H:%M"), e.format("%H:%M")),
            _ => String::new(),
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {}{range}  ", g.event), Style::default().fg(WARNING)),
            Span::raw(event.title.clone()),
        ])));
    }
    items
}

fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3), Constraint::Length(3)])
        .split(frame.area());

    let g = Glyphs::new(app.nerd);

    // Header: brand mark, pane tab strip, and a credit link, sharing one rule.
    let header = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(11), Constraint::Min(0), Constraint::Length(18)])
        .split(chunks[0]);
    let underline = Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray));
    let brand = Paragraph::new(Line::from(Span::styled(
        " Dayboard",
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )))
    .block(underline.clone());
    frame.render_widget(brand, header[0]);

    let selected_tab = match app.pane {
        Pane::Tasks => 0,
        Pane::Events => 1,
        Pane::Calendar => 2,
    };
    let tabs = Tabs::new(vec![
        format!("{}Tasks", g.tab_tasks),
        format!("{}Events", g.tab_events),
        format!("{}Calendar", g.tab_calendar),
    ])
    .select(selected_tab)
    .style(Style::default().fg(Color::Gray))
    .highlight_style(Style::default().fg(Color::White).bg(ACCENT).add_modifier(Modifier::BOLD))
    .divider(Span::styled("  ", Style::default()))
    .padding(" ", " ")
    .block(underline.clone());
    frame.render_widget(tabs, header[1]);

    let credit = Paragraph::new(Line::from(Span::styled(
        "subhajitpaul.com ",
        Style::default().fg(Color::DarkGray),
    )))
    .alignment(Alignment::Right)
    .block(underline);
    frame.render_widget(credit, header[2]);

    match app.pane {
        Pane::Tasks => {
            if app.rows.is_empty() {
                render_empty(frame, chunks[1], pane_block("Tasks"), "No tasks yet", "Press 'a' to add your first task");
            } else {
                let items: Vec<ListItem> = app
                    .rows
                    .iter()
                    .map(|row| {
                        let indent = if row.depth > 0 { "  \u{21b3} " } else { "" };
                        let check = if row.task.done { g.check_on } else { g.check_off };
                        let style = if row.task.done {
                            Style::default().add_modifier(Modifier::CROSSED_OUT).fg(Color::DarkGray)
                        } else {
                            Style::default()
                        };
                        let mut spans = vec![Span::styled(format!("{indent}{check}{}", row.task.title), style)];
                        let count = app.reminder_counts.get(&row.task.id).copied().unwrap_or(0);
                        if count > 0 {
                            let label = if app.nerd {
                                format!("  {}{count}", g.bell)
                            } else if count == 1 {
                                " (reminder)".to_string()
                            } else {
                                format!(" ({count} reminders)")
                            };
                            spans.push(Span::styled(label, Style::default().fg(WARNING)));
                        }
                        ListItem::new(Line::from(spans))
                    })
                    .collect();

                let mut state = ListState::default();
                state.select(Some(app.selected));

                let list = List::new(items)
                    .block(pane_block("Tasks"))
                    .highlight_style(Style::default().bg(ACCENT).fg(Color::White).add_modifier(Modifier::BOLD))
                    .highlight_symbol(" ");

                frame.render_stateful_widget(list, chunks[1], &mut state);
            }
        }
        Pane::Events => {
            if app.events.is_empty() {
                render_empty(frame, chunks[1], pane_block("Events"), "No events yet", "Press 'v' to add an event");
            } else {
                let list = List::new(events_items(app)).block(pane_block("Events"));
                frame.render_widget(list, chunks[1]);
            }
        }
        Pane::Calendar => draw_calendar(frame, chunks[1], app),
    }

    let (bottom_text, bottom_border_color): (Text, Color) = match &app.mode {
        Mode::Normal => {
            if app.status.is_empty() {
                (
                    Text::from(Span::styled(contextual_help(app.pane), Style::default().fg(Color::DarkGray))),
                    Color::DarkGray,
                )
            } else {
                (Text::from(app.status.clone()), ACCENT)
            }
        }
        Mode::Input { buffer, .. } => (
            Text::from(Line::from(vec![
                Span::styled("> ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::raw(buffer.clone()),
                Span::styled("\u{2588}", Style::default().fg(ACCENT)),
            ])),
            ACCENT,
        ),
        Mode::ConfirmDelete { has_children, .. } => {
            let msg = if *has_children {
                "delete task and its subtasks? y/n"
            } else {
                "delete task? y/n"
            };
            (Text::from(Span::styled(msg, Style::default().fg(DANGER).add_modifier(Modifier::BOLD))), DANGER)
        }
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
    let g = Glyphs::new(app.nerd);
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
            Span::styled(format!("  {}", g.event), Style::default().fg(ACCENT)),
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
        let suffix = if app.nerd { String::new() } else { " (reminder)".to_string() };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {}", g.bell), Style::default().fg(WARNING)),
            Span::raw(format!("{title}  ")),
            Span::styled(format!("{time}{suffix}"), Style::default().fg(WARNING)),
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

/// A hand-rolled Month grid: a full 7-wide, 6-tall grid of roomy day cells
/// (big day numbers + colored event/reminder dots), replacing ratatui's tiny
/// built-in `Monthly` widget so the calendar stays legible at small terminal
/// font sizes.
fn draw_month(frame: &mut Frame, area: Rect, app: &App) {
    let block = accent_block(format!(" Calendar \u{b7} {} ", app.cal_cursor.format("%B %Y")));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // one weekday-header row + six equal week rows.
    let mut vconstraints = vec![Constraint::Length(1)];
    vconstraints.extend(vec![Constraint::Ratio(1, 6); 6]);
    let rows = Layout::vertical(vconstraints).split(inner);
    let hconstraints = vec![Constraint::Ratio(1, 7); 7];

    let head_cols = Layout::horizontal(hconstraints.clone()).split(rows[0]);
    for (i, name) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
        // left-aligned with a leading space to line up with the day numbers
        // below (which render as " {day}"), not centered.
        let head = Paragraph::new(Line::from(Span::styled(
            format!(" {name}"),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(head, head_cols[i]);
    }

    let days = month_days(app.cal_cursor);
    let today = Local::now().date_naive();
    for w in 0..6 {
        let cols = Layout::horizontal(hconstraints.clone()).split(rows[w + 1]);
        for (d, col) in cols.iter().enumerate() {
            draw_month_cell(frame, *col, app, days[w * 7 + d], today);
        }
    }
}

fn draw_month_cell(frame: &mut Frame, area: Rect, app: &App, day: NaiveDate, today: NaiveDate) {
    let in_month = day.month() == app.cal_cursor.month();
    let is_today = day == today;
    let is_cursor = day == app.cal_cursor;

    // only the selected day and today get a box, so the grid stays uncluttered.
    let content_area = if is_cursor || is_today {
        let color = if is_cursor { ACCENT } else { WARNING };
        let mut style = Style::default().fg(color);
        if is_cursor {
            style = style.add_modifier(Modifier::BOLD);
        }
        let cell = Block::default().borders(Borders::ALL).border_style(style);
        let inner = cell.inner(area);
        frame.render_widget(cell, area);
        inner
    } else {
        area
    };

    let num_style = if is_today {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else if in_month {
        Style::default()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let events = app.events.iter().filter(|e| to_naive_date(e.start_at) == day).count();
    let reminders = app.reminders.iter().filter(|(r, _)| to_naive_date(r.remind_at) == day).count();

    let mut lines = vec![Line::from(Span::styled(format!(" {}", day.day()), num_style))];
    let mut dots: Vec<Span> = vec![Span::raw(" ")];
    for _ in 0..events.min(5) {
        dots.push(Span::styled("\u{2022}", Style::default().fg(ACCENT)));
    }
    for _ in 0..reminders.min(3) {
        dots.push(Span::styled("\u{2022}", Style::default().fg(WARNING)));
    }
    if dots.len() > 1 {
        lines.push(Line::from(dots));
    }
    frame.render_widget(Paragraph::new(lines), content_area);
}

/// An accent-bordered focused block with a dynamic title (calendar views).
fn accent_block(title: String) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(title, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
}

fn draw_calendar(frame: &mut Frame, area: Rect, app: &App) {
    match app.cal_scale {
        CalScale::Month => draw_month(frame, area, app),
        CalScale::Week => {
            let title = format!(" Week of {} ", week_start(app.cal_cursor).format("%b %-d"));
            let list = List::new(week_items(app)).block(accent_block(title));
            frame.render_widget(list, area);
        }
        CalScale::Day => {
            let title = format!(" {} ", app.cal_cursor.format("%A, %b %-d"));
            let items = calendar_items_for_day(app, app.cal_cursor);
            if items.is_empty() {
                render_empty(frame, area, accent_block(title), "Nothing scheduled", "Press Enter to add an event on this day");
            } else {
                frame.render_widget(List::new(items).block(accent_block(title)), area);
            }
        }
    }
}
