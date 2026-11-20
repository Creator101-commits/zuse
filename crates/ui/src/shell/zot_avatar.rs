//! Zapper avatars in the shell: the shared renderer, and choosing a picture
//! of your own for a Zapper's settings page.
use super::*;
use zuse_protocol::zots::Zot;
pub(super) use zuse_viewport::zapper_avatar::ZapperFace;

/// A Zapper's profile picture, `size` logical pixels across.
pub(super) fn zot_mascot(face: &ZapperFace, size: f32) -> gpui::Div {
    zuse_viewport::zapper_avatar::zapper_face(face, size)
}

/// What a saved Zapper looks like: its picture, else its robot.
pub(super) fn zot_avatar(zot: &Zot) -> ZapperFace {
    ZapperFace::of(zot)
}

impl Shell {
    /// Ask for a picture, then crop and shrink it off the main thread. The
    /// result lands on the open settings page; saving the page stores it with
    /// the Zapper on its home device, which every client reads it from.
    pub(super) fn upload_zot_picture(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use image".into()),
        });
        cx.spawn(async move |this, cx| {
            let Some(path) = paths
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|paths| paths.into_iter().next())
            else {
                return;
            };
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    let bytes =
                        std::fs::read(&path).map_err(|_| "Zuse couldn't open that file.")?;
                    zuse_viewport::zapper_avatar::prepare_image(&bytes)
                })
                .await;
            let _ = this.update(cx, |this, cx| match prepared {
                Ok(url) => {
                    if let Some(form) = this.zot_ui.page.as_mut() {
                        form.image = Some(url);
                    }
                    cx.notify();
                }
                Err(text) => this.show_notice(crate::toast::ToastKind::Error, text, cx),
            });
        })
        .detach();
    }
}

impl Shell {
    /// Open a Zapper's settings page in the isolated native fixture.
    #[doc(hidden)]
    pub fn fixture_open_zot_editor(&mut self, zot_id: &str, cx: &mut Context<Self>) {
        let zot = self
            .state
            .read(cx)
            .zots
            .iter()
            .find(|zot| zot.id == zot_id)
            .cloned();
        if let Some(zot) = zot {
            self.open_zot_editor(zot, cx);
        }
    }
}
