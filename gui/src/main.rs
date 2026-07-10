mod calendar;
mod icons;
mod theme;

use std::collections::HashMap;

use caldav_core::{Db, Event, Reminder, Task};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input, Space};
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

/// Accepts "today", "tomorrow" (case-insensitive), or an ISO date. Empty
/// input also means "today" since that's the composer's default.
fn parse_event_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    match input.trim().to_lowercase().as_str() {
        "" | "today" => Some(today),
        "tomorrow" => Some(today + Duration::days(1)),
        _ => NaiveDate::parse_from_str(input.trim(), "%Y-%m-%d").ok(),
    }
}

/// Accepts 24-hour ("15:00") or 12-hour ("3pm", "3:00 PM", "3 PM") input,
/// so a non-technical user typing however they'd naturally say it still
/// works — not just one exact strftime format.
fn parse_event_time(input: &str) -> Option<NaiveTime> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(t) = NaiveTime::parse_from_str(trimmed, "%H:%M") {
        return Some(t);
    }
    // chrono's "%I%p" alone fails to parse ("NotEnough") — insert ":00" so
    // "3pm" becomes "3:00PM" and always goes through the one format that works.
    let mut compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_uppercase();
    if !compact.contains(':')
        && let Some(pos) = compact.find(['A', 'P'])
    {
        compact.insert_str(pos, ":00");
    }
    NaiveTime::parse_from_str(&compact, "%I:%M%p").ok()
}

fn to_epoch(date: NaiveDate, time: NaiveTime) -> Option<i64> {
    Local.from_local_datetime(&NaiveDateTime::new(date, time)).single().map(|dt| dt.timestamp())
}

/// Next quarter-hour from now, e.g. 14:12 -> "14:15" — a ready-to-submit
/// default so most users never have to type a time at all.
fn default_time_str() -> String {
    let now = Local::now().time();
    let mins_into_hour = now.minute() % 15;
    let rounded = now + Duration::minutes((15 - mins_into_hour) as i64 % 15);
    rounded.format("%H:%M").to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DurationPreset {
    ThirtyMin,
    #[default]
    OneHour,
    TwoHours,
    AllDay,
}

impl DurationPreset {
    const ALL: [DurationPreset; 4] = [Self::ThirtyMin, Self::OneHour, Self::TwoHours, Self::AllDay];

    fn label(self) -> &'static str {
        match self {
            Self::ThirtyMin => "30 min",
            Self::OneHour => "1 hour",
            Self::TwoHours => "2 hours",
            Self::AllDay => "All day",
        }
    }

    /// None means "all day", handled separately since it isn't a fixed offset
    /// from a start time.
    fn minutes(self) -> Option<i64> {
        match self {
            Self::ThirtyMin => Some(30),
            Self::OneHour => Some(60),
            Self::TwoHours => Some(120),
            Self::AllDay => None,
        }
    }
}

#[cfg(test)]
mod event_composer_tests {
    use super::*;

    #[test]
    fn date_accepts_today_tomorrow_and_iso() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        assert_eq!(parse_event_date("today", today), Some(today));
        assert_eq!(parse_event_date("Today", today), Some(today));
        assert_eq!(parse_event_date("", today), Some(today));
        assert_eq!(parse_event_date("tomorrow", today), Some(today + Duration::days(1)));
        assert_eq!(parse_event_date("2026-08-01", today), NaiveDate::from_ymd_opt(2026, 8, 1));
        assert_eq!(parse_event_date("not a date", today), None);
    }

    #[test]
    fn time_accepts_24h_and_12h_variants() {
        let expect = NaiveTime::from_hms_opt(15, 30, 0).unwrap();
        assert_eq!(parse_event_time("15:30"), Some(expect));
        assert_eq!(parse_event_time("3:30pm"), Some(expect));
        assert_eq!(parse_event_time("3:30 PM"), Some(expect));
        assert_eq!(parse_event_time("3:30PM"), Some(expect));
        let three_flat = NaiveTime::from_hms_opt(15, 0, 0).unwrap();
        assert_eq!(parse_event_time("3pm"), Some(three_flat));
        assert_eq!(parse_event_time("3 PM"), Some(three_flat));
        assert_eq!(parse_event_time(""), None);
        assert_eq!(parse_event_time("not a time"), None);
    }
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
    event_date: String,
    event_time: String,
    event_duration: DurationPreset,
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
            event_date: "Today".to_string(),
            event_time: default_time_str(),
            event_duration: DurationPreset::default(),
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
    AuthComplete(Result<(), String>),
    SwitchView(View),
    EventTitleChanged(String),
    EventDateChanged(String),
    EventTimeChanged(String),
    EventDurationChanged(DurationPreset),
    SubmitEvent,
    DeleteEvent(i64),
    SystemThemeChanged(iced::theme::Mode),
    CalScaleChanged(calendar::CalendarScale),
    CalPrev,
    CalNext,
    CalToday,
    SelectDay(NaiveDate),
    OpenSite,
}

const SITE_URL: &str = "https://subhajitpaul.com";

fn do_sync(app: &mut App) {
    match caldav_core::sync::run(&app.db) {
        Ok(()) => {
            app.status = "sync complete".to_string();
            app.refresh();
        }
        Err(e) => app.status = format!("sync failed: {e}"),
    }
}

/// Runs a blocking closure on its own thread and resolves once it's done,
/// so it can back an `iced::Task` without blocking the UI thread — used for
/// `auth::authenticate()`, which blocks until the user finishes in their
/// browser.
fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("auth worker thread panicked") }
}

fn update(app: &mut App, message: Message) -> IcedTask<Message> {
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
            if caldav_core::auth::is_authenticated() {
                do_sync(app);
            } else {
                app.status = "opening your browser to connect Google\u{2026}".to_string();
                return IcedTask::perform(run_blocking(|| caldav_core::auth::authenticate().map_err(|e| e.to_string())), Message::AuthComplete);
            }
        }
        Message::AuthComplete(result) => match result {
            Ok(()) => do_sync(app),
            Err(e) => app.status = format!("Google sign-in failed: {e}"),
        },
        Message::SwitchView(v) => app.view = v,
        Message::EventTitleChanged(s) => app.event_title = s,
        Message::EventDateChanged(s) => app.event_date = s,
        Message::EventTimeChanged(s) => app.event_time = s,
        Message::EventDurationChanged(d) => app.event_duration = d,
        Message::SubmitEvent => {
            let title = app.event_title.trim().to_string();
            if title.is_empty() {
                app.status = "event needs a title".to_string();
                return IcedTask::none();
            }
            let today = Local::now().date_naive();
            let Some(date) = parse_event_date(&app.event_date, today) else {
                app.status = "couldn't understand that date \u{2014} try \u{201c}today\u{201d}, \u{201c}tomorrow\u{201d}, or YYYY-MM-DD".to_string();
                return IcedTask::none();
            };
            let range = if app.event_duration == DurationPreset::AllDay {
                to_epoch(date, NaiveTime::MIN).zip(to_epoch(date + Duration::days(1), NaiveTime::MIN))
            } else {
                match parse_event_time(&app.event_time) {
                    None => {
                        app.status = "couldn't understand that time \u{2014} try \u{201c}3:00 PM\u{201d} or \u{201c}15:00\u{201d}".to_string();
                        return IcedTask::none();
                    }
                    Some(time) => to_epoch(date, time)
                        .map(|start_at| (start_at, start_at + app.event_duration.minutes().expect("checked above") * 60)),
                }
            };
            match range {
                None => app.status = "that date/time doesn't exist locally (daylight saving change?)".to_string(),
                Some((start_at, end_at)) => {
                    let _ = app.db.create_event(&title, start_at, end_at);
                    app.event_title.clear();
                    app.event_date = "Today".to_string();
                    app.event_time = default_time_str();
                    app.event_duration = DurationPreset::default();
                    app.status = "event added".to_string();
                    app.refresh();
                }
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
            app.event_date = day.format("%Y-%m-%d").to_string();
        }
        Message::OpenSite => {
            // fire-and-forget; if xdg-open is missing there's simply no browser.
            let _ = std::process::Command::new("xdg-open").arg(SITE_URL).spawn();
        }
    }
    IcedTask::none()
}

fn sidebar(app: &App) -> Element<'_, Message> {
    let th = app.theme();
    let muted = theme::muted_color(&th);

    let nav = |bytes: &'static [u8], label: &'static str, target: View| {
        let selected = app.view == target;
        let tint = if selected { theme::on_accent() } else { muted };
        let font = if selected { theme::semibold() } else { iced::Font::DEFAULT };
        button(
            row![
                icons::icon(bytes, 20.0, tint),
                text(label).size(theme::SIZE_BODY).font(font),
            ]
            .spacing(theme::SM)
            .align_y(Alignment::Center),
        )
        .style(theme::nav_item(selected))
        .padding([theme::SM, theme::MD])
        .width(Length::Fill)
        .on_press(Message::SwitchView(target))
    };

    let brand = container(
        row![
            icons::icon(icons::CALENDAR, 22.0, theme::on_accent()),
            text("Dayboard").size(theme::SIZE_TITLE).font(theme::bold()),
        ]
        .spacing(theme::XS)
        .align_y(Alignment::Center),
    )
    .style(theme::brand)
    .padding([theme::SM, theme::MD])
    .width(Length::Fill);

    let sync = button(
        row![
            icons::icon(icons::SYNC, 18.0, theme::on_accent()),
            text("Sync").size(theme::SIZE_BODY),
        ]
        .spacing(theme::XS)
        .align_y(Alignment::Center),
    )
    .style(theme::primary_button)
    .padding([theme::SM, theme::MD])
    .width(Length::Fill)
    .on_press(Message::SyncNow);

    let credit = column![
        text("Made by Subhajit Paul").size(theme::SIZE_CAPTION).style(theme::muted_text),
        button(text("subhajitpaul.com").size(theme::SIZE_CAPTION))
            .style(theme::link_button)
            .padding(0)
            .on_press(Message::OpenSite),
    ]
    .spacing(theme::XXS)
    .align_x(Alignment::Center)
    .width(Length::Fill);

    let col = column![
        brand,
        Space::new().height(theme::SM),
        nav(icons::TASKS, "Tasks", View::Tasks),
        nav(icons::EVENTS, "Events", View::Events),
        nav(icons::CALENDAR, "Calendar", View::Calendar),
        Space::new().height(Length::Fill),
        status_line(app),
        sync,
        Space::new().height(theme::XS),
        credit,
    ]
    .spacing(theme::XXS)
    .padding(theme::MD)
    .width(Length::Fixed(224.0))
    .height(Length::Fill);

    container(col).style(theme::sidebar).height(Length::Fill).into()
}

fn page_header<'a>(title: &'a str, subtitle: String) -> Element<'a, Message> {
    column![
        text(title).size(theme::SIZE_DISPLAY).font(theme::bold()),
        text(subtitle).size(theme::SIZE_META).style(theme::muted_text),
    ]
    .spacing(theme::XXS)
    .into()
}

fn empty_state<'a>(illustration: &'static [u8], title: &'a str, hint: &'a str) -> Element<'a, Message> {
    container(
        column![
            icons::illustration(illustration, 220.0),
            text(title).size(theme::SIZE_TITLE).font(theme::semibold()),
            text(hint).size(theme::SIZE_META).style(theme::muted_text),
        ]
        .spacing(theme::SM)
        .align_x(Alignment::Center),
    )
    .padding(theme::XL)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
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
    let reminder_count = app.reminder_counts.get(&task_id).copied().unwrap_or(0);
    let th = app.theme();
    let muted = theme::muted_color(&th);
    let warning = th.extended_palette().warning.base.color;

    let title = if r.task.done {
        text(r.task.title.clone()).size(theme::SIZE_BODY).style(theme::muted_text)
    } else {
        text(r.task.title.clone()).size(theme::SIZE_BODY)
    };

    let mut content = row![Space::new().width(r.depth as f32 * 20.0)]
        .spacing(theme::SM)
        .align_y(Alignment::Center);

    content = content.push(
        checkbox(r.task.done)
            .on_toggle(move |_| Message::ToggleDone(task_id))
            .style(theme::checkbox_style)
            .size(20)
            .spacing(theme::SM),
    );
    content = content.push(title);
    if reminder_count > 0 {
        content = content.push(
            row![
                icons::icon(icons::BELL, 14.0, warning),
                text(reminder_count.to_string()).size(theme::SIZE_CAPTION).style(theme::warning_text),
            ]
            .spacing(theme::XXS)
            .align_y(Alignment::Center),
        );
    }
    content = content.push(Space::new().width(Length::Fill));
    content = content.push(
        button(icons::icon(icons::PLUS, 16.0, muted))
            .style(theme::ghost_button)
            .padding(theme::XS)
            .on_press(Message::SetParent(task_id)),
    );
    content = content.push(
        button(icons::icon(icons::TRASH, 16.0, muted))
            .style(theme::danger_ghost_button)
            .padding(theme::XS)
            .on_press(Message::DeleteTask(task_id)),
    );

    content.into()
}

/// Stacks each item in its own elevated card. The card look is the app's
/// primary list treatment (Tasks, Events, calendar Day view).
fn card_list<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut col = column![].spacing(theme::XS);
    for item in items {
        col = col.push(
            container(item)
                .style(theme::card)
                .padding([theme::SM, theme::MD])
                .width(Length::Fill),
        );
    }
    // small inset so card shadows aren't clipped by the scroll viewport edges.
    scrollable(column![col].padding([theme::XXS, theme::XXS]))
        .height(Length::Fill)
        .into()
}

fn add_button(label: &str, msg: Message) -> Element<'_, Message> {
    button(
        row![
            icons::icon(icons::PLUS, 16.0, theme::on_accent()),
            text(label.to_string()).size(theme::SIZE_BODY),
        ]
        .spacing(theme::XS)
        .align_y(Alignment::Center),
    )
    .style(theme::primary_button)
    .padding([theme::XS, theme::MD])
    .on_press(msg)
    .into()
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
            text(format!("Adding subtask to: {label}")).size(theme::SIZE_CAPTION).style(theme::accent_text).into()
        }
        None => Space::new().height(0.0).into(),
    };

    let mut input_row = row![
        text_input("Add a task\u{2026}", &app.input)
            .on_input(Message::InputChanged)
            .on_submit(Message::Submit)
            .style(theme::input)
            .padding(theme::SM),
        add_button("Add", Message::Submit),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center);

    if app.adding_parent.is_some() {
        input_row = input_row.push(
            button(text("Cancel")).style(theme::ghost_button).padding([theme::XS, theme::MD]).on_press(Message::ClearParent),
        );
    }

    container(column![breadcrumb, input_row].spacing(theme::XS))
        .padding(theme::SM)
        .width(Length::Fill)
        .into()
}

fn tasks_view(app: &App) -> Element<'_, Message> {
    let open = app.rows.iter().filter(|r| !r.task.done).count();
    let done = app.rows.len() - open;
    let subtitle = format!("{open} open \u{b7} {done} done");

    let list = if app.rows.is_empty() {
        empty_state(icons::ILL_TASKS, "No tasks yet", "Add your first task below to get started")
    } else {
        card_list(app.rows.iter().map(|r| task_row(app, r)).collect())
    };

    column![page_header("Tasks", subtitle), task_composer(app), list]
        .spacing(theme::MD)
        .padding(theme::LG)
        .into()
}

fn event_row<'a>(app: &'a App, event: &'a Event) -> Element<'a, Message> {
    let event_id = event.id;
    let th = app.theme();
    let accent = theme::accent_color(&th);
    let muted = theme::muted_color(&th);
    row![
        icons::icon(icons::CLOCK, 18.0, accent),
        column![
            text(event.title.clone()).size(theme::SIZE_BODY).font(theme::semibold()),
            text(format_range(event.start_at, event.end_at)).size(theme::SIZE_META).style(theme::muted_text),
        ]
        .spacing(theme::XXS),
        Space::new().width(Length::Fill),
        button(icons::icon(icons::TRASH, 16.0, muted))
            .style(theme::danger_ghost_button)
            .padding(theme::XS)
            .on_press(Message::DeleteEvent(event_id)),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center)
    .into()
}

fn event_composer(app: &App) -> Element<'_, Message> {
    let today = Local::now().date_naive();
    let date_valid = parse_event_date(&app.event_date, today).is_some();
    let time_valid = app.event_duration == DurationPreset::AllDay || parse_event_time(&app.event_time).is_some();

    let title_input = text_input("Event title", &app.event_title)
        .on_input(Message::EventTitleChanged)
        .on_submit(Message::SubmitEvent)
        .style(theme::input)
        .padding(theme::SM)
        .width(Length::Fill);

    let date_input = text_input("Today", &app.event_date)
        .on_input(Message::EventDateChanged)
        .on_submit(Message::SubmitEvent)
        .style(if date_valid { theme::input } else { theme::input_invalid })
        .padding(theme::SM)
        .width(Length::Fixed(130.0));

    let time_input = text_input("e.g. 3:00 PM", &app.event_time)
        .on_input(Message::EventTimeChanged)
        .on_submit(Message::SubmitEvent)
        .style(if time_valid { theme::input } else { theme::input_invalid })
        .padding(theme::SM)
        .width(Length::Fixed(130.0));

    let mut duration_row = row![].spacing(theme::XXS);
    for preset in DurationPreset::ALL {
        duration_row = duration_row.push(
            button(text(preset.label()).size(theme::SIZE_META))
                .style(theme::segment(app.event_duration == preset))
                .padding([theme::XS, theme::SM])
                .on_press(Message::EventDurationChanged(preset)),
        );
    }

    let fields_row = row![date_input, time_input, duration_row, add_button("Add", Message::SubmitEvent)]
        .spacing(theme::SM)
        .align_y(Alignment::Center);

    let hint = if app.event_duration == DurationPreset::AllDay {
        "Date can be \u{201c}today\u{201d}, \u{201c}tomorrow\u{201d}, or YYYY-MM-DD \u{2014} all-day events skip the time."
    } else {
        "Date can be \u{201c}today\u{201d}, \u{201c}tomorrow\u{201d}, or YYYY-MM-DD; time can be \u{201c}3pm\u{201d} or \u{201c}15:00\u{201d}."
    };

    container(
        column![
            title_input,
            fields_row,
            text(hint).size(theme::SIZE_CAPTION).style(theme::muted_text),
        ]
        .spacing(theme::SM),
    )
    .padding(theme::SM)
    .width(Length::Fill)
    .into()
}

fn events_view(app: &App) -> Element<'_, Message> {
    let subtitle = match app.events.len() {
        1 => "1 event".to_string(),
        n => format!("{n} events"),
    };
    let list = if app.events.is_empty() {
        empty_state(icons::ILL_EVENTS, "No events yet", "Add one below \u{2014} synced events appear here too")
    } else {
        card_list(app.events.iter().map(|e| event_row(app, e)).collect())
    };

    column![page_header("Events", subtitle), event_composer(app), list]
        .spacing(theme::MD)
        .padding(theme::LG)
        .into()
}

fn view(app: &App) -> Element<'_, Message> {
    let body = match app.view {
        View::Tasks => tasks_view(app),
        View::Events => events_view(app),
        View::Calendar => calendar::view(app),
    };

    row![
        sidebar(app),
        container(body).width(Length::Fill).height(Length::Fill),
    ]
    .height(Length::Fill)
    .into()
}

pub fn main() -> iced::Result {
    iced::application(App::new, update, view)
        .title("Dayboard")
        .theme(|app: &App| app.theme())
        .subscription(|_| iced::system::theme_changes().map(Message::SystemThemeChanged))
        .run()
}
