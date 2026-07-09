//! Calendar preview: Day/Week/Month grid views, layered on top of the
//! existing Events data. Pure date math lives here standalone; the widget
//! tree (added once this scaffolding is wired up) reaches back into
//! `main.rs`'s private `App`/`Message`/helpers, which Rust privacy allows
//! since this is a descendant module of the crate root.

use caldav_core::Reminder;
use chrono::{Datelike, Duration, Local, Months, NaiveDate, TimeZone};
use iced::widget::{button, column, container, grid, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

use crate::{theme, App, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarScale {
    Day,
    Week,
    Month,
}

/// The Monday that starts the week containing `date`.
pub fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// 42 consecutive days (6 Monday-first weeks) covering the month containing
/// `cursor`, including leading/trailing days from adjacent months.
pub fn month_grid(cursor: NaiveDate) -> Vec<NaiveDate> {
    let first_of_month = cursor.with_day(1).expect("day 1 is always valid");
    let start = week_start(first_of_month);
    (0..42).map(|i| start + Duration::days(i)).collect()
}

/// Moves `cursor` by one unit of `scale`, forward or backward.
pub fn shift(cursor: NaiveDate, scale: CalendarScale, forward: bool) -> NaiveDate {
    match scale {
        CalendarScale::Day => cursor + Duration::days(if forward { 1 } else { -1 }),
        CalendarScale::Week => cursor + Duration::days(if forward { 7 } else { -7 }),
        CalendarScale::Month => {
            if forward {
                cursor.checked_add_months(Months::new(1)).unwrap_or(cursor)
            } else {
                cursor.checked_sub_months(Months::new(1)).unwrap_or(cursor)
            }
        }
    }
}

fn to_local_date(ts: i64) -> NaiveDate {
    Local.timestamp_opt(ts, 0).single().expect("valid unix timestamp").date_naive()
}

fn period_label(cursor: NaiveDate, scale: CalendarScale) -> String {
    match scale {
        CalendarScale::Day => cursor.format("%A, %B %-d, %Y").to_string(),
        CalendarScale::Week => {
            let start = week_start(cursor);
            let end = start + Duration::days(6);
            format!("{} \u{2013} {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
        }
        CalendarScale::Month => cursor.format("%B %Y").to_string(),
    }
}

fn nav_bar(app: &App) -> Element<'_, Message> {
    let scale_tab = |label: &'static str, target: CalendarScale| {
        button(text(label).size(theme::SIZE_BODY))
            .style(theme::tab_button(app.cal_scale == target))
            .padding([theme::XS, theme::MD])
            .on_press(Message::CalScaleChanged(target))
    };

    row![
        button(text("<").size(theme::SIZE_BODY)).style(theme::ghost_button).on_press(Message::CalPrev),
        button(text("Today").size(theme::SIZE_BODY)).style(theme::ghost_button).on_press(Message::CalToday),
        button(text(">").size(theme::SIZE_BODY)).style(theme::ghost_button).on_press(Message::CalNext),
        text(period_label(app.cal_cursor, app.cal_scale)).size(theme::SIZE_BODY),
        Space::new().width(Length::Fill),
        scale_tab("Day", CalendarScale::Day),
        scale_tab("Week", CalendarScale::Week),
        scale_tab("Month", CalendarScale::Month),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center)
    .into()
}

fn reminder_row<'a>(reminder: &'a Reminder, title: &'a str) -> Element<'a, Message> {
    let time = Local
        .timestamp_opt(reminder.remind_at, 0)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_default();
    row![
        text(title).size(theme::SIZE_BODY),
        Space::new().width(Length::Fill),
        text(time).size(theme::SIZE_META).style(theme::warning_text),
    ]
    .spacing(theme::SM)
    .align_y(Alignment::Center)
    .padding([theme::XS, 0.0])
    .into()
}

fn month_cell(app: &App, day: NaiveDate, today: NaiveDate) -> Element<'_, Message> {
    let in_month = day.month() == app.cal_cursor.month();
    let day_number: Element<'_, Message> = if in_month {
        text(day.day().to_string()).size(theme::SIZE_BODY).into()
    } else {
        text(day.day().to_string()).size(theme::SIZE_BODY).style(theme::muted_text).into()
    };

    let event_count = app.events.iter().filter(|e| to_local_date(e.start_at) == day).count();
    let reminder_count = app.reminders.iter().filter(|(r, _)| to_local_date(r.remind_at) == day).count();

    let mut cell_col = column![day_number].spacing(theme::XXS);
    if event_count > 0 {
        let label = if event_count == 1 { "1 event".to_string() } else { format!("{event_count} events") };
        cell_col = cell_col.push(text(label).size(theme::SIZE_CAPTION));
    }
    if reminder_count > 0 {
        let label = if reminder_count == 1 { "1 reminder".to_string() } else { format!("{reminder_count} reminders") };
        cell_col = cell_col.push(text(label).size(theme::SIZE_CAPTION).style(theme::warning_text));
    }

    let cell = container(cell_col).padding(theme::XXS).width(Length::Fill).height(Length::Fill);
    let cell = if day == today { cell.style(theme::tinted_container) } else { cell };

    button(cell)
        .style(theme::ghost_button)
        .padding(0)
        .width(Length::Fill)
        .height(Length::Fill)
        .on_press(Message::SelectDay(day))
        .into()
}

fn month_view(app: &App) -> Element<'_, Message> {
    let weekday_header = row(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].into_iter().map(|d| {
        text(d).size(theme::SIZE_CAPTION).style(theme::muted_text).width(Length::Fill).into()
    }))
    .spacing(theme::XXS);

    let today = Local::now().date_naive();
    let mut cal_grid = grid(std::iter::empty()).columns(7).spacing(theme::XXS).height(Length::Fill);
    for day in month_grid(app.cal_cursor) {
        cal_grid = cal_grid.push(month_cell(app, day, today));
    }

    column![weekday_header, cal_grid].spacing(theme::XS).into()
}

fn week_day_column(app: &App, day: NaiveDate) -> Element<'_, Message> {
    let header = button(text(day.format("%a %-d").to_string()).size(theme::SIZE_CAPTION))
        .style(theme::tab_button(day == app.cal_cursor))
        .on_press(Message::SelectDay(day));

    let mut col = column![header].spacing(theme::XXS);
    for event in app.events.iter().filter(|e| to_local_date(e.start_at) == day) {
        col = col.push(crate::event_row(event));
    }
    for (reminder, title) in app.reminders.iter().filter(|(r, _)| to_local_date(r.remind_at) == day) {
        col = col.push(reminder_row(reminder, title));
    }

    container(scrollable(col)).width(Length::FillPortion(1)).height(Length::Fill).into()
}

fn week_view(app: &App) -> Element<'_, Message> {
    let start = week_start(app.cal_cursor);
    let columns: Vec<Element<'_, Message>> =
        (0..7).map(|i| week_day_column(app, start + Duration::days(i))).collect();

    row(columns).spacing(theme::SM).height(Length::Fill).into()
}

fn day_view(app: &App) -> Element<'_, Message> {
    let day = app.cal_cursor;
    let day_events: Vec<Element<'_, Message>> =
        app.events.iter().filter(|e| to_local_date(e.start_at) == day).map(crate::event_row).collect();
    let day_reminders: Vec<(&Reminder, &String)> =
        app.reminders.iter().filter(|(r, _)| to_local_date(r.remind_at) == day).map(|(r, t)| (r, t)).collect();

    let events_section = if day_events.is_empty() {
        crate::empty_state("No events", "Add one below")
    } else {
        crate::divided_list(day_events)
    };

    let mut sections = column![events_section].spacing(theme::MD);
    if !day_reminders.is_empty() {
        let mut rem_col =
            column![text("Reminders").size(theme::SIZE_META).style(theme::muted_text)].spacing(theme::XXS);
        for (reminder, title) in &day_reminders {
            rem_col = rem_col.push(reminder_row(reminder, title));
        }
        sections = sections.push(rem_col);
    }

    column![crate::event_composer(app), sections].spacing(theme::MD).into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let body = match app.cal_scale {
        CalendarScale::Day => day_view(app),
        CalendarScale::Week => week_view(app),
        CalendarScale::Month => month_view(app),
    };

    column![nav_bar(app), body].spacing(theme::MD).padding(theme::MD).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    #[test]
    fn week_start_is_monday() {
        // 2026-07-10 is a Friday.
        let d = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        let ws = week_start(d);
        assert_eq!(ws.weekday(), Weekday::Mon);
        assert_eq!(ws, NaiveDate::from_ymd_opt(2026, 7, 6).unwrap());
    }

    #[test]
    fn month_grid_covers_first_and_last_of_month() {
        let cursor = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let grid = month_grid(cursor);
        assert_eq!(grid.len(), 42);
        assert!(grid.contains(&NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()));
        assert!(grid.contains(&NaiveDate::from_ymd_opt(2026, 7, 31).unwrap()));
        assert_eq!(grid[0].weekday(), Weekday::Mon);
    }

    #[test]
    fn shift_month_rolls_over_year_boundary() {
        let dec = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        let jan = shift(dec, CalendarScale::Month, true);
        assert_eq!(jan, NaiveDate::from_ymd_opt(2027, 1, 15).unwrap());
        assert_eq!(shift(jan, CalendarScale::Month, false), dec);
    }

    #[test]
    fn shift_day_and_week() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap();
        assert_eq!(shift(d, CalendarScale::Day, true), NaiveDate::from_ymd_opt(2026, 7, 11).unwrap());
        assert_eq!(shift(d, CalendarScale::Week, true), NaiveDate::from_ymd_opt(2026, 7, 17).unwrap());
    }
}
