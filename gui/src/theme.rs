//! Design tokens: an Adwaita-flavored accent (GNOME's `#3584e4`) layered on
//! two custom `iced::Theme`s so light/dark can track the OS. iced's own
//! `Theme::Light`/`Dark` don't use this exact accent, which is why we build
//! our own instead of reusing them.
//!
//! Beyond the palette this module carries the whole visual system — elevation
//! (cards + shadows), a gradient accent, the sidebar chrome, and styled
//! inputs/checkboxes/chips — so widgets never hardcode a color or radius.

use iced::gradient::Linear;
use iced::widget::{button, checkbox, container, text_input};
use iced::{
    color, Background, Border, Color, Gradient, Radians, Shadow, Theme, Vector,
};

// --- spacing scale (px) ---
pub const XXS: f32 = 4.0;
pub const XS: f32 = 8.0;
pub const SM: f32 = 12.0;
pub const MD: f32 = 16.0;
pub const LG: f32 = 24.0;
pub const XL: f32 = 32.0;

// --- corner radii (px) ---
pub const RADIUS_SM: f32 = 6.0;
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_LG: f32 = 16.0;
pub const RADIUS_PILL: f32 = 999.0;

// --- type scale (px) ---
pub const SIZE_DISPLAY: f32 = 26.0;
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

pub fn semibold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Semibold,
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

// --- theme-adaptive surfaces -------------------------------------------------

/// Whether the active theme is a dark one (by background luminance). Lets us
/// pick elevated surfaces and shadow strengths that read on both themes.
pub fn is_dark(theme: &Theme) -> bool {
    let bg = theme.extended_palette().background.base.color;
    (bg.r + bg.g + bg.b) / 3.0 < 0.5
}

/// The raised surface color for cards/inputs — brighter than the page so
/// elevated elements separate from the background.
pub fn surface_color(theme: &Theme) -> Color {
    if is_dark(theme) {
        color!(0x303030)
    } else {
        Color::WHITE
    }
}

/// The sidebar's own (recessed) surface color.
fn sidebar_color(theme: &Theme) -> Color {
    if is_dark(theme) {
        color!(0x1c1c1c)
    } else {
        color!(0xeeeef1)
    }
}

/// Icon tint that matches muted body text.
pub fn muted_color(theme: &Theme) -> Color {
    theme.extended_palette().background.strong.color
}

pub fn accent_color(theme: &Theme) -> Color {
    theme.extended_palette().primary.base.color
}

pub fn on_accent() -> Color {
    Color::WHITE
}

/// The signature accent gradient (bright → deep blue), used for the brand mark,
/// the active nav item, and primary buttons. Accent is identical in both
/// themes, so the stops are constant.
pub fn accent_gradient() -> Gradient {
    Gradient::Linear(
        Linear::new(Radians(2.2))
            .add_stop(0.0, color!(0x4a93ea))
            .add_stop(1.0, color!(0x2b6fca)),
    )
}

fn shadow(theme: &Theme, alpha: f32, y: f32, blur: f32) -> Shadow {
    let a = if is_dark(theme) { alpha * 2.4 } else { alpha };
    Shadow {
        color: Color { a, ..Color::BLACK },
        offset: Vector::new(0.0, y),
        blur_radius: blur,
    }
}

// --- containers --------------------------------------------------------------

/// Elevated card: a raised surface with a hairline border and a soft shadow.
/// The default surface for list rows and grouped content.
pub fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(surface_color(theme))),
        border: Border {
            radius: RADIUS_MD.into(),
            width: 1.0,
            color: palette.background.strong.color.scale_alpha(0.35),
        },
        shadow: shadow(theme, 0.08, 1.0, 6.0),
        text_color: None,
        snap: true,
    }
}

/// Sidebar chrome: a recessed vertical surface.
pub fn sidebar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(sidebar_color(theme))),
        ..container::Style::default()
    }
}

/// The gradient brand block at the top of the sidebar.
pub fn brand(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Gradient(accent_gradient())),
        border: Border {
            radius: RADIUS_MD.into(),
            ..Border::default()
        },
        text_color: Some(Color::WHITE),
        ..container::Style::default()
    }
}

/// Weak-tinted, rounded container used for composer bars.
pub fn tinted_container(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.background.weak.color)),
        border: Border {
            radius: RADIUS_LG.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A small solid dot of `color`, used as a calendar day indicator.
pub fn dot(fill: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(fill)),
        border: Border {
            radius: RADIUS_PILL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The accent bar shown on the "today" calendar cell / selected day header.
pub fn today_cell(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(Background::Color(palette.primary.base.color.scale_alpha(0.12))),
        border: Border {
            radius: RADIUS_MD.into(),
            width: 1.5,
            color: palette.primary.base.color.scale_alpha(0.5),
        },
        ..container::Style::default()
    }
}

// --- buttons -----------------------------------------------------------------

/// Filled primary action (Add, Sync): gradient fill, white text, soft shadow
/// that lifts on hover.
pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        background: Some(Background::Gradient(accent_gradient())),
        text_color: Color::WHITE,
        border: Border {
            radius: RADIUS_MD.into(),
            ..Border::default()
        },
        shadow: shadow(theme, 0.22, 2.0, 8.0),
        snap: true,
    };
    match status {
        button::Status::Hovered => button::Style {
            shadow: shadow(theme, 0.30, 4.0, 12.0),
            ..base
        },
        button::Status::Pressed => button::Style {
            shadow: shadow(theme, 0.15, 1.0, 4.0),
            ..base
        },
        button::Status::Disabled => button::Style {
            background: Some(Background::Color(
                theme.extended_palette().background.strong.color,
            )),
            shadow: Shadow::default(),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Low-emphasis "ghost" button for secondary row actions — no chrome at rest,
/// a faint tint on hover.
pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => {
            Some(Background::Color(palette.background.strong.color.scale_alpha(0.3)))
        }
        _ => None,
    };
    button::Style {
        background,
        text_color: palette.background.base.text,
        border: Border {
            radius: RADIUS_SM.into(),
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
                radius: RADIUS_SM.into(),
                ..Border::default()
            },
            ..button::Style::default()
        },
        _ => ghost_button(theme, status),
    }
}

/// A sidebar navigation item. Selected = gradient accent pill with white text
/// and an accent glow; unselected = transparent with a hover tint.
pub fn nav_item(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        if selected {
            button::Style {
                background: Some(Background::Gradient(accent_gradient())),
                text_color: Color::WHITE,
                border: Border {
                    radius: RADIUS_MD.into(),
                    ..Border::default()
                },
                shadow: shadow(theme, 0.28, 2.0, 10.0),
                snap: true,
            }
        } else {
            let background = match status {
                button::Status::Hovered | button::Status::Pressed => {
                    Some(Background::Color(palette.background.strong.color.scale_alpha(0.35)))
                }
                _ => None,
            };
            button::Style {
                background,
                text_color: palette.background.base.text,
                border: Border {
                    radius: RADIUS_MD.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        }
    }
}

/// A text link: accent-colored, no chrome, brightening on hover.
pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.strong.color,
        _ => palette.primary.base.color,
    };
    button::Style {
        background: None,
        text_color,
        border: Border {
            radius: RADIUS_SM.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// Segmented-control tab (calendar Day/Week/Month, week-day headers).
pub fn segment(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        if selected {
            button::Style {
                background: Some(Background::Color(palette.primary.base.color)),
                text_color: Color::WHITE,
                border: Border {
                    radius: RADIUS_SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        } else {
            ghost_button(theme, status)
        }
    }
}

// --- inputs ------------------------------------------------------------------

/// Text input with a raised surface and an accent focus ring.
pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let palette = theme.extended_palette();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(surface_color(theme)),
        border: Border {
            radius: RADIUS_MD.into(),
            width: if focused { 2.0 } else { 1.0 },
            color: if focused {
                palette.primary.base.color
            } else {
                palette.background.strong.color.scale_alpha(0.5)
            },
        },
        icon: palette.background.strong.color,
        placeholder: palette.background.strong.color,
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    }
}

pub fn checkbox_style(theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let palette = theme.extended_palette();
    let checked = matches!(
        status,
        checkbox::Status::Active { is_checked: true }
            | checkbox::Status::Hovered { is_checked: true }
            | checkbox::Status::Disabled { is_checked: true }
    );
    let hovered = matches!(status, checkbox::Status::Hovered { .. });
    let border_color = if checked || hovered {
        palette.primary.base.color
    } else {
        palette.background.strong.color
    };
    checkbox::Style {
        background: if checked {
            Background::Color(palette.primary.base.color)
        } else {
            Background::Color(surface_color(theme))
        },
        icon_color: Color::WHITE,
        border: Border {
            radius: RADIUS_SM.into(),
            width: 2.0,
            color: border_color,
        },
        text_color: None,
    }
}

// --- text --------------------------------------------------------------------

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

pub fn accent_text(theme: &Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(theme.extended_palette().primary.base.color),
    }
}
