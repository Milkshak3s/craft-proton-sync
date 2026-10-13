//! Colours and shapes, after ArtCraft Launcher and getartcraft.com: hard square corners and 1 px
//! hairlines instead of shadows. Widgets read [`Tokens::get`]; nothing hard-codes a colour.

use egui::{Color32, CornerRadius, Stroke, Theme, Visuals};

/// Colour tokens for one appearance, by the launcher's names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Page surface.
    pub bg: Color32,
    /// Wells and badges.
    pub sunken: Color32,
    /// Body text.
    pub ink: Color32,
    /// Headings.
    pub ink_strong: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Labels.
    pub faint: Color32,
    /// Hairlines.
    pub line: Color32,
    /// Outlined buttons.
    pub line_strong: Color32,
    pub accent: Color32,
    pub accent_ink: Color32,
    /// Primary buttons.
    pub invert_bg: Color32,
    pub invert_fg: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

/// `ink` at `alpha` over `bg`, as an opaque colour.
const fn mix(ink: u32, bg: u32, alpha: u32) -> Color32 {
    const fn channel(ink: u8, bg: u8, alpha: u32) -> u8 {
        ((ink as u32 * alpha + bg as u32 * (255 - alpha)) / 255) as u8
    }
    let ([_, ir, ig, ib], [_, br, bg, bb]) = (ink.to_be_bytes(), bg.to_be_bytes());
    Color32::from_rgb(channel(ir, br, alpha), channel(ig, bg, alpha), channel(ib, bb, alpha))
}

impl Tokens {
    pub const DARK: Self = Self {
        dark: true,
        bg: hex(0x121316),
        sunken: hex(0x060607),
        ink: hex(0xf2f1ee),
        ink_strong: hex(0xffffff),
        muted: mix(0xf2f1ee, 0x121316, 153),
        faint: mix(0xf2f1ee, 0x121316, 97),
        line: mix(0xf2f1ee, 0x121316, 38),
        line_strong: mix(0xf2f1ee, 0x121316, 102),
        accent: hex(0x2d81ff),
        accent_ink: hex(0x74aaff),
        invert_bg: hex(0xffffff),
        invert_fg: hex(0x0b0b0c),
        good: hex(0x3fd6b8),
        warn: hex(0xffc14d),
        bad: hex(0xff6259),
    };

    pub const LIGHT: Self = Self {
        dark: false,
        bg: hex(0xf2f1ee),
        sunken: hex(0xe9e8e4),
        ink: hex(0x101014),
        ink_strong: hex(0x000000),
        muted: mix(0x101014, 0xf2f1ee, 158),
        faint: mix(0x101014, 0xf2f1ee, 107),
        line: mix(0x101014, 0xf2f1ee, 41),
        line_strong: mix(0x101014, 0xf2f1ee, 102),
        accent: hex(0x2d81ff),
        accent_ink: hex(0x1659c4),
        invert_bg: hex(0x101014),
        invert_fg: hex(0xf2f1ee),
        good: hex(0x0a7563),
        warn: hex(0x8a5800),
        bad: hex(0xb3261e),
    };

    pub fn get(ctx: &egui::Context) -> Self {
        if ctx.theme() == Theme::Dark { Self::DARK } else { Self::LIGHT }
    }

    pub fn hairline(&self) -> Stroke {
        Stroke::new(1.0, self.line)
    }
}

fn visuals(t: &Tokens) -> Visuals {
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = t.bg;
    v.window_fill = t.bg;
    v.extreme_bg_color = t.sunken;
    v.faint_bg_color = t.sunken;
    v.override_text_color = Some(t.ink);
    v.hyperlink_color = t.accent_ink;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.0, t.invert_bg);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_stroke = t.hairline();
    for (w, line) in [
        (&mut v.widgets.noninteractive, t.line),
        (&mut v.widgets.inactive, t.line_strong),
        (&mut v.widgets.hovered, t.ink),
        (&mut v.widgets.active, t.ink),
        (&mut v.widgets.open, t.ink),
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::new(1.0, line);
        w.bg_fill = t.bg;
        w.weak_bg_fill = t.bg;
        w.fg_stroke = Stroke::new(1.0, t.ink);
        w.expansion = 0.0;
    }
    v.widgets.hovered.weak_bg_fill = t.sunken;
    v.widgets.hovered.bg_fill = t.sunken;
    v
}

/// Sets both appearances; the window follows the system's light or dark setting.
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&Tokens::DARK));
    ctx.set_visuals_of(Theme::Light, visuals(&Tokens::LIGHT));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
    });
}
