//! Installation analytics preference, owned and persisted by the connected engine.
use super::widgets;
use crate::{state::AppState, theme::Theme};
use gpui::{prelude::*, *};

pub(super) struct AnalyticsCard {
    state: Entity<AppState>,
    status: Option<zuse_analytics::Status>,
    busy: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    engine: Option<crate::state::EngineHandle>,
}

impl AnalyticsCard {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut card = Self {
            state,
            status: None,
            busy: false,
            error: None,
            task: None,
            engine: None,
        };
        card.request(None, cx);
        card
    }

    fn request(&mut self, enabled: Option<bool>, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        self.engine = Some(engine.clone());
        self.busy = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let method = if enabled.is_some() {
                zuse_rpc::methods::SET_ANALYTICS_SETTINGS
            } else {
                zuse_rpc::methods::GET_ANALYTICS_SETTINGS
            };
            let result = engine
                .client()
                .call(method, serde_json::json!({"enabled": enabled}))
                .await
                .map_err(|e| e.to_string())
                .and_then(|value| serde_json::from_value(value).map_err(|e| e.to_string()));
            this.update(cx, |card, cx| {
                card.busy = false;
                match result {
                    Ok(status) => card.status = Some(status),
                    Err(error) => {
                        card.error = Some(error);
                        card.status = None;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

impl Render for AnalyticsCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.state.read(cx).engine().cloned();
        let changed = match (&connected, &self.engine) {
            (Some(next), Some(previous)) => !next.same_connection(previous),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.task = None;
            self.busy = false;
            self.status = None;
            self.error = None;
            self.engine = connected;
            if self.engine.is_some() {
                self.request(None, cx);
            }
        }
        let theme = Theme::of(cx).for_settings_surface();
        let enabled = self.status.as_ref().is_some_and(|s| s.enabled);
        let interactive = self.status.is_some() && !self.busy;
        widgets::group(&theme, "Privacy", "")
            .child(widgets::section_card(&theme).mt_0()
                .child(widgets::card_row(&theme, true)
                    .child(widgets::row_copy(&theme, "Share usage analytics",
                        "Share pseudonymous product usage. Prompts, code, paths, and credentials are excluded."))
                    .child(widgets::toggle_switch(&theme, enabled, "analytics-enabled")
                        .id("analytics-enabled").role(Role::Switch).aria_label("Share usage analytics")
                        .aria_toggled(if enabled { Toggled::True } else { Toggled::False })
                        .when(interactive, |el| el.cursor_pointer().tab_index(0)
                            .on_click(cx.listener(move |card, _, _, cx| card.request(Some(!enabled), cx))))
                        .when(!interactive, |el| el.opacity(0.5)))))
            .when_some(self.error.clone(), |el, error| el.child(widgets::error_strip(&theme, error)))
    }
}
