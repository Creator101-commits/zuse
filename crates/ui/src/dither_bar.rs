//! Usage and load bars drawn in the app's ordered-dither style: the same 2px
//! Bayer cells as the send key and the usage meter, densest at the fill's
//! leading edge, over a sparse dotted track.
use crate::theme::Theme;
use gpui::{Hsla, ParentElement, Styled, div, px};

/// The bar's default height: three rows of 2px cells.
pub(crate) const DITHER_BAR_HEIGHT: f32 = 6.0;

/// How loaded a resource is, by the fraction in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Load {
    Normal,
    Elevated,
    Critical,
}

/// Above 70% a resource is elevated, above 90% critical.
pub(crate) fn load_level(fraction: f32) -> Load {
    if fraction > 0.9 {
        Load::Critical
    } else if fraction > 0.7 {
        Load::Elevated
    } else {
        Load::Normal
    }
}

/// The theme's success, warning or danger tone for a load.
pub(crate) fn load_color(fraction: f32, theme: &Theme) -> Hsla {
    match load_level(fraction) {
        Load::Normal => theme.success,
        Load::Elevated => theme.warning,
        Load::Critical => theme.danger,
    }
}

/// The track's dust: the muted ink, faint enough for both themes.
fn track_color(theme: &Theme) -> Hsla {
    theme.text_muted.opacity(0.3)
}

/// A full-width dithered bar filled to `fraction` in `fill`. Callers can
/// override its width or height; it paints whatever box it gets.
pub(crate) fn dither_bar(fraction: f32, fill: Hsla, theme: &Theme) -> gpui::Div {
    div()
        .relative()
        .flex_none()
        .w_full()
        .h(px(DITHER_BAR_HEIGHT))
        .child(zuse_viewport::dither::meter(
            fraction.clamp(0.0, 1.0),
            fill,
            track_color(theme),
        ))
}

/// A machine resource bar (CPU, memory, disk): green, amber past 70%, red
/// past 90%.
pub(crate) fn load_bar(fraction: f32, theme: &Theme) -> gpui::Div {
    dither_bar(fraction, load_color(fraction, theme), theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zuse_viewport::dither::{MeterCell, lit_columns, meter_cell};

    #[test]
    fn thresholds_pick_success_warning_and_danger() {
        assert_eq!(load_level(0.0), Load::Normal);
        assert_eq!(load_level(0.7), Load::Normal);
        assert_eq!(load_level(0.71), Load::Elevated);
        assert_eq!(load_level(0.9), Load::Elevated);
        assert_eq!(load_level(0.95), Load::Critical);
        for theme in [Theme::dark(), Theme::light()] {
            assert_eq!(load_color(0.2, &theme), theme.success);
            assert_eq!(load_color(0.8, &theme), theme.warning);
            assert_eq!(load_color(1.0, &theme), theme.danger);
        }
    }

    #[test]
    fn cells_light_up_to_the_percent() {
        // A 60px bar has 30 columns of 2px cells.
        for (percent, lit) in [(0.0, 0), (0.001, 1), (0.25, 8), (0.5, 15), (1.0, 30)] {
            assert_eq!(lit_columns(percent, 30), lit, "{percent}");
            for y in 0..3 {
                for x in 0..30 {
                    let filled = matches!(meter_cell(x, y, lit), MeterCell::On | MeterCell::Faint);
                    assert_eq!(filled, x < lit, "{percent} at ({x}, {y})");
                }
            }
        }
    }
}
