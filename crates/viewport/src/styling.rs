//! Shared application action treatment for native and browser surfaces.
use gpui::{prelude::*, *};

/// Corner radius of a labelled action.
const DITHER_RADIUS: f32 = 6.;

/// The app's dithered key with a label: olive when `primary`, grey
/// otherwise. The texture stays calm behind the label so it always reads.
pub fn dither_action(
    id: impl Into<ElementId>,
    name: impl Into<SharedString>,
    enabled: bool,
    primary: bool,
) -> Stateful<Div> {
    let name = name.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(name.clone())
        .relative()
        .h(px(28.))
        .px_3()
        .rounded(px(DITHER_RADIUS))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        // The dark plate keeps the white label crisp; the texture stays calm
        // behind it (see `dither::label_fill`).
        .text_color(rgb(0xffffff))
        .bg(crate::palette::tone(
            hsla(80. / 360., 0.18, 0.075, 1.),
            hsla(80. / 360., 0.28, 0.2, 1.),
        ))
        .border_1()
        .border_color(if primary {
            hsla(83. / 360., 0.5, 0.45, 0.45)
        } else {
            hsla(0., 0., 0.6, 0.25)
        })
        .when(!enabled, |d| d.opacity(0.45))
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|d| d.opacity(0.92))
                .active(|d| d.opacity(0.8))
        })
        .child(crate::dither::label_fill(
            if primary {
                [151, 183, 76]
            } else {
                [120, 124, 130]
            },
            DITHER_RADIUS,
        ))
        .child(
            div()
                .relative()
                .font_weight(FontWeight::SEMIBOLD)
                .child(name),
        )
}

pub fn round_icon_action(
    id: impl Into<ElementId>,
    glyph: &str,
    label: &'static str,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .relative()
        .size(px(28.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(0x303230))
        .when(!enabled, |d| d.opacity(0.45))
        .when(enabled, |d| d.cursor_pointer().hover(|d| d.opacity(0.85)))
        .child(
            svg()
                .path(SharedString::from(format!("icons/{glyph}.svg")))
                .size(px(14.))
                .text_color(rgb(0xffffff)),
        )
}
