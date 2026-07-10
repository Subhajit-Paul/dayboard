//! SVG assets embedded at compile time. Symbolic line icons are tinted at
//! runtime via iced's svg color filter (`svg::Style.color`) so they follow the
//! theme; the empty-state illustrations bake their own accent colors (with
//! `fill-opacity`, so they read on both light and dark) and are left untinted.

use iced::widget::svg::{self, Svg};
use iced::{Color, Element};

macro_rules! icon_bytes {
    ($($name:ident => $file:literal),* $(,)?) => {
        $(pub const $name: &[u8] = include_bytes!(concat!("../assets/icons/", $file, ".svg"));)*
    };
}

icon_bytes! {
    TASKS => "tasks",
    EVENTS => "events",
    CALENDAR => "calendar",
    SYNC => "sync",
    PLUS => "plus",
    TRASH => "trash",
    CHEVRON_LEFT => "chevron-left",
    CHEVRON_RIGHT => "chevron-right",
    BELL => "bell",
    TODAY => "today",
    CLOCK => "clock",
}

pub const ILL_TASKS: &[u8] = include_bytes!("../assets/illustrations/empty-tasks.svg");
pub const ILL_EVENTS: &[u8] = include_bytes!("../assets/illustrations/empty-events.svg");
pub const ILL_CALENDAR: &[u8] = include_bytes!("../assets/illustrations/empty-calendar.svg");

/// A theme-tinted symbolic icon rendered at `size` × `size` px.
pub fn icon<'a, M: 'a>(bytes: &'static [u8], size: f32, color: Color) -> Element<'a, M> {
    Svg::new(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(move |_theme, _status| svg::Style { color: Some(color) })
        .into()
}

/// A baked-color illustration, scaled to `width` (height follows aspect ratio).
pub fn illustration<'a, M: 'a>(bytes: &'static [u8], width: f32) -> Element<'a, M> {
    Svg::new(svg::Handle::from_memory(bytes)).width(width).into()
}
