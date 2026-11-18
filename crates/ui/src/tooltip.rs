//! Shared tooltip appearance for the native host.

use crate::theme::Theme;
use gpui::{AnyElement, App, Context, Div, SharedString, Window, div, prelude::*, px};

pub use zuse_viewport::tooltip::{
    GAP, MAX_WIDTH, PAD_X, PAD_Y, Placement, RADIUS, SIDE_GAP, TEXT_SIZE, TooltipExt, text_tooltip,
};

/// How long the pointer rests on a row before its details card opens.
pub const CARD_DELAY: std::time::Duration = std::time::Duration::from_millis(450);
/// A details card's width.
pub const CARD_WIDTH: f32 = 288.0;

/// The bare chip surface — border, fill, radius, padding and type — for
/// tooltips that lay out richer content than one line of text. Wrap the
/// finished card in [`frost`].
pub fn surface(theme: &Theme) -> Div {
    div()
        .px(px(PAD_X))
        .py(px(PAD_Y))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(theme.border)
        .bg(crate::popover::surface_bg(theme))
        .font_family(theme.font_sans.clone())
        .text_size(px(TEXT_SIZE))
        .text_color(theme.text_strong.opacity(0.92))
}

/// Backdrop blur behind a [`surface`] card.
pub fn frost(card: impl IntoElement) -> AnyElement {
    crate::frost::frosted(RADIUS, crate::frost::MENU_BLUR, card).into_any_element()
}

/// A finished one-line chip reading `text`.
pub fn chip(text: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    frost(
        surface(theme)
            .max_w(px(MAX_WIDTH))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(text.into()),
    )
}

/// A details card: the chip's border, fill and type, sized for several
/// lines. Fill it with a title, [`card_line`]s and a footnote, then wrap it
/// in [`frost`].
pub fn card(theme: &Theme) -> Div {
    surface(theme)
        .w(px(CARD_WIDTH))
        .px(px(10.0))
        .py(px(8.0))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .text_size(px(12.0))
        .font_weight(gpui::FontWeight::NORMAL)
}

/// One icon-led line in a [`card`], ellipsized to the card's width.
pub fn card_line(glyph: impl IntoElement, text: impl IntoElement, theme: &Theme) -> Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .text_color(theme.text_muted)
        .child(
            div()
                .size(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(glyph),
        )
        .child(div().flex_1().min_w_0().truncate().child(text))
}

/// Hover cards: a details card that opens beside a row after a short rest
/// and closes when the pointer leaves, a click lands, or the list scrolls.
pub trait HoverCardExt: TooltipExt {
    /// Open `build`'s card to the right of this element (left near the
    /// window edge) after [`CARD_DELAY`]. `build` runs only when the card
    /// opens, so it can read current state instead of capturing it.
    fn hover_card(self, build: impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static) -> Self {
        self.anchored_tooltip(Placement::Right { width: CARD_WIDTH }, build)
            .tooltip_show_delay(CARD_DELAY)
    }
}

impl<E: TooltipExt> HoverCardExt for E {}

/// A one-line hover note in the shared chip.
pub struct TextTooltip {
    text: SharedString,
}

impl TextTooltip {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl Render for TextTooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        chip(self.text.clone(), Theme::of(cx))
    }
}

/// Install the native theme-aware renderer for tooltips in shared views.
pub fn install(cx: &mut App) {
    cx.set_global(zuse_viewport::tooltip::TooltipRenderer(|text, _, cx| {
        cx.new(|_| TextTooltip::new(text)).into()
    }));
}
