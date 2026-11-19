//! Form pieces shared by the Environments and Machine pages: titled sections
//! whose descriptions stay visible, bordered script fields and a save bar.
use super::*;

/// A titled card. Unlike `cs::group`, the description is always visible.
pub(super) fn section(
    title: &'static str,
    description: &'static str,
    action: Option<AnyElement>,
    body: impl IntoElement,
) -> Div {
    crate::cloud_settings::kit::section(title, description, action, body)
}

/// Label, hint and a bordered script editor.
pub(super) fn script_field(
    label: &'static str,
    hint: &'static str,
    input: Entity<TextInput>,
) -> Div {
    column()
        .gap(px(6.))
        .child(crate::cloud_settings::kit::title_with_tip(label, hint))
        .child(script_box(input))
}

pub(super) fn script_box(input: Entity<TextInput>) -> Div {
    div()
        .w_full()
        .min_h(px(72.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(7.))
        .border_1()
        .border_color(palette::rgb(BORDER))
        .bg(palette::tone(
            hsla(0., 0., 0.06, 1.),
            hsla(0., 0., 0.98, 1.),
        ))
        .child(input)
}

/// A one-line bordered input that takes the space it is given.
pub(super) fn line_field(input: Entity<TextInput>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(30.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .border_1()
        .border_color(palette::rgb(BORDER))
        .bg(palette::tone(
            hsla(0., 0., 0.06, 1.),
            hsla(0., 0., 0.98, 1.),
        ))
        .child(input)
}

/// Quiet copy for an empty list inside a section.
pub(super) fn empty(text: &'static str) -> Div {
    div()
        .px(px(12.))
        .py(px(14.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(palette::rgb(MUTED))
        .child(text)
}

pub(super) fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family("Geist Mono")
        .text_size(px(11.5))
        .line_height(px(16.))
        .text_color(palette::rgb(MUTED))
        .truncate()
        .child(text.into())
}

pub(super) fn message(text: &str, error: bool) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(if error {
            s::destructive_text()
        } else {
            palette::rgb(MUTED).into()
        })
        .child(text.to_owned())
}

/// The floating "Unsaved changes" bar with Reset and Save, shown only while
/// the page has edits. Cmd/Ctrl+S saves too (see `SaveChanges`).
pub(super) fn save_bar(
    id: &'static str,
    viewport: Size<Pixels>,
    dirty: bool,
    busy: bool,
    discard: impl Fn(&mut RepositoryPanel, &mut Context<RepositoryPanel>) + 'static,
    save: impl Fn(&mut RepositoryPanel, &mut Context<RepositoryPanel>) + 'static,
    cx: &mut Context<RepositoryPanel>,
) -> AnyElement {
    if !dirty {
        return div().into_any_element();
    }
    use crate::cloud_settings::kit;
    let reset = kit::reset_button(SharedString::from(format!("{id}-discard")), !busy)
        .when(!busy, |b| {
            b.on_click(cx.listener(move |this, _, _, cx| discard(this, cx)))
        });
    let save = kit::primary_button(
        SharedString::from(format!("{id}-save")),
        if busy { "Saving…" } else { "Save" },
        !busy,
    )
    .when(!busy, |b| {
        b.on_click(cx.listener(move |this, _, _, cx| save(this, cx)))
    });
    kit::unsaved_bar(viewport, reset, save).into_any_element()
}
