//! Design tokens: an Adwaita-flavored accent (GNOME's `#3584e4`) layered on
//! two custom `iced::Theme`s so light/dark can track the OS. iced's own
//! `Theme::Light`/`Dark` don't use this exact accent, which is why we build
//! our own instead of reusing them.

use iced::widget::{button, checkbox, container, rule};
use iced::{color, Background, Border, Color, Theme};

// --- spacing scale (px) ---
pub const XXS: f32 = 4.0;
pub const XS: f32 = 8.0;
pub const SM: f32 = 12.0;
pub const MD: f32 = 16.0;
#[allow(dead_code)] // reserved for larger section gaps as the app grows
pub const LG: f32 = 24.0;
pub const XL: f32 = 32.0;

// --- type scale (px) ---
pub const SIZE_TITLE: f32 = 20.0;
pub const SIZE_BODY: f32 = 15.0;
pub const SIZE_META: f32 = 13.0;
pub const SIZE_CAPTION: f32 = 12.0;

pub fn bold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Bold,
        ..iced::Font::DEFAULT
    }
}

pub fn adwaita_light() -> Theme {
    Theme::custom(
        "Adwaita Light".to_string(),
        iced::theme::Palette {
            background: color!(0xfafafa),
            text: color!(0x2e3436),
            primary: color!(0x3584e4),
            success: color!(0x2ec27e),
            warning: color!(0xe5a50a),
            danger: color!(0xe01b24),
        },
    )
}

pub fn adwaita_dark() -> Theme {
    Theme::custom(
        "Adwaita Dark".to_string(),
        iced::theme::Palette {
            background: color!(0x242424),
            text: color!(0xeeeeec),
            primary: color!(0x3584e4),
            success: color!(0x33d17a),
            warning: color!(0xe5a50a),
            danger: color!(0xed333b),
        },
    )
}

/// Low-emphasis "ghost" button used for secondary row actions (e.g. add
/// subtask) — no chrome at rest, a faint tint on hover.
pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered => Some(Background::Color(palette.background.weak.color)),
        _ => None,
    };
    button::Style {
        background,
        text_color: palette.background.base.text,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Same as [`ghost_button`] but tints danger-red on hover/press, for
/// destructive row actions (delete).
pub fn danger_ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Background::Color(palette.danger.weak.color)),
            text_color: palette.danger.weak.text,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        },
        _ => ghost_button(theme, status),
    }
}

/// Selected/unselected tab button in the header view-switcher.
pub fn tab_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if selected {
            button::primary(theme, status)
        } else {
            button::text(theme, status)
        }
    }
}

/// Weak-tinted, rounded container used for the header bar and composer bar.
pub fn tinted_container(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        border: Border {
            radius: 8.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn muted_text(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.extended_palette().background.strong.color),
    }
}

pub fn warning_text(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.extended_palette().warning.base.color),
    }
}

pub fn checkbox_style(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    checkbox::primary(theme, status)
}

pub fn divider(theme: &Theme) -> rule::Style {
    rule::weak(theme)
}

#[allow(dead_code)] // for danger accents outside of buttons (e.g. validation text), added as tokens land
pub fn danger_color(theme: &Theme) -> Color {
    theme.extended_palette().danger.base.color
}
