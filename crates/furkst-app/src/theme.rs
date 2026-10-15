//! Colors and visual style, in light and dark variants.

use egui::{Color32, Visuals};
use furkst_core::{ChangeKind, RefKind};

use crate::settings::ThemeChoice;

/// Commit graph lane colors, chosen to stay distinguishable on both backgrounds.
const LANES: [Color32; 8] = [
    Color32::from_rgb(0x2f, 0x81, 0xf7),
    Color32::from_rgb(0xe3, 0x6a, 0x2e),
    Color32::from_rgb(0x2d, 0xa4, 0x4e),
    Color32::from_rgb(0xc9, 0x3c, 0x9a),
    Color32::from_rgb(0x8a, 0x63, 0xd2),
    Color32::from_rgb(0xd2, 0x99, 0x22),
    Color32::from_rgb(0x1b, 0xa3, 0xa8),
    Color32::from_rgb(0xd7, 0x3a, 0x49),
];

pub fn lane(index: usize) -> Color32 {
    LANES[index % LANES.len()]
}

/// A stable color for an arbitrary string, used for avatars.
pub fn hashed(text: &str) -> Color32 {
    let hash = text.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
    lane(hash)
}

pub fn change(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked => Color32::from_rgb(0x2d, 0xa4, 0x4e),
        ChangeKind::Modified | ChangeKind::TypeChange => Color32::from_rgb(0xd2, 0x99, 0x22),
        ChangeKind::Deleted => Color32::from_rgb(0xd7, 0x3a, 0x49),
        ChangeKind::Renamed | ChangeKind::Copied => Color32::from_rgb(0x2f, 0x81, 0xf7),
        ChangeKind::Conflicted => Color32::from_rgb(0xc9, 0x3c, 0x9a),
    }
}

pub fn reference(kind: RefKind) -> Color32 {
    match kind {
        RefKind::Head => Color32::from_rgb(0xd7, 0x3a, 0x49),
        RefKind::LocalBranch => Color32::from_rgb(0x2f, 0x81, 0xf7),
        RefKind::RemoteBranch => Color32::from_rgb(0x8a, 0x63, 0xd2),
        RefKind::Tag => Color32::from_rgb(0xd2, 0x99, 0x22),
        RefKind::Stash => Color32::from_rgb(0x6e, 0x76, 0x81),
    }
}

/// Background tints for diff lines.
pub struct DiffColors {
    pub added: Color32,
    pub removed: Color32,
    pub hunk: Color32,
    pub selected: Color32,
}

impl DiffColors {
    pub fn of(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self {
                added: Color32::from_rgb(0x1d, 0x35, 0x26),
                removed: Color32::from_rgb(0x41, 0x22, 0x26),
                hunk: Color32::from_rgb(0x22, 0x2c, 0x3a),
                selected: Color32::from_rgb(0x26, 0x4f, 0x78),
            }
        } else {
            Self {
                added: Color32::from_rgb(0xe6, 0xff, 0xec),
                removed: Color32::from_rgb(0xff, 0xeb, 0xe9),
                hunk: Color32::from_rgb(0xdd, 0xf4, 0xff),
                selected: Color32::from_rgb(0xb6, 0xd6, 0xfb),
            }
        }
    }
}

pub fn apply(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.interaction.selectable_labels = false;
    });
}

/// WCAG AA contrast for normal-size text.
const READABLE: f32 = 4.5;
/// WCAG AAA contrast, for small text that must read at a glance, like ref badges.
const VERY_READABLE: f32 = 7.0;
/// WCAG contrast for shapes that aren't text, like outlines.
const VISIBLE: f32 = 3.0;

/// WCAG contrast ratio between two colors, from 1 (identical) to 21 (black on white).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let luminance = |c: Color32| {
        let channel = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.039_28 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let channel = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

/// `color`, moved step by step toward `toward` until its contrast against `other` reaches `ratio`.
fn readable(mut color: Color32, other: Color32, toward: Color32, ratio: f32) -> Color32 {
    for _ in 0..20 {
        if contrast(color, other) >= ratio {
            break;
        }
        color = mix(color, toward, 0.1);
    }
    color
}

/// A fill in the hue of `color` that white text reads well on (checked-out branch badges, status letters, avatars).
pub fn solid(color: Color32) -> Color32 {
    readable(color, Color32::WHITE, Color32::BLACK, READABLE)
}

/// How a ref badge in the colors of `color` is painted.
#[derive(Clone, Copy, Debug)]
pub struct BadgeColors {
    pub fill: Color32,
    pub text: Color32,
    pub outline: Color32,
}

/// A ref badge's colors: a light wash of `color` with a shade of it as text (`strong`: a deep fill with white text, for
/// the checked-out branch), always outlined. Text reads at AAA contrast and the outline stands out from the panel, in
/// either theme.
pub fn badge(color: Color32, strong: bool, visuals: &Visuals) -> BadgeColors {
    // Away from the panel: lighter in the dark theme, darker in the light one.
    let toward = if visuals.dark_mode { Color32::WHITE } else { Color32::BLACK };
    let outline = readable(color, visuals.panel_fill, toward, VISIBLE);
    if strong {
        let fill = readable(color, Color32::WHITE, Color32::BLACK, VERY_READABLE);
        return BadgeColors { fill, text: Color32::WHITE, outline };
    }
    let fill = mix(visuals.panel_fill, color, if visuals.dark_mode { 0.24 } else { 0.12 });
    BadgeColors { fill, text: readable(color, fill, toward, VERY_READABLE), outline }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_color() -> Vec<Color32> {
        let refs = [RefKind::Head, RefKind::LocalBranch, RefKind::RemoteBranch, RefKind::Tag, RefKind::Stash].map(reference);
        let changes = [ChangeKind::Modified, ChangeKind::Added, ChangeKind::Deleted, ChangeKind::Renamed, ChangeKind::Conflicted]
            .map(change);
        refs.into_iter().chain(changes).chain(LANES).collect()
    }

    #[test]
    fn every_badge_reads_well_in_both_themes() {
        for visuals in [Visuals::light(), Visuals::dark()] {
            for color in every_color() {
                for strong in [false, true] {
                    let badge = badge(color, strong, &visuals);
                    let at = format!("{color:?}, strong={strong}, dark={}", visuals.dark_mode);
                    assert!(contrast(badge.text, badge.fill) >= VERY_READABLE, "text, {at}");
                    assert!(contrast(badge.outline, visuals.panel_fill) >= VISIBLE, "outline, {at}");
                }
                assert!(contrast(Color32::WHITE, solid(color)) >= READABLE, "{color:?} solid");
            }
        }
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::RED, Color32::RED) - 1.0).abs() < 0.01);
    }
}
