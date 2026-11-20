//! New Zapper page from offline data: several projects on two devices.
//! Usage: `zapper-create-fixture <capture directory>`; writes `create.png`,
//! and with the searchable picker also `search.png`, `no-match.png` and
//! `advanced.png`.
use gpui::{AnyWindowHandle, AppContext, AsyncApp, Bounds, WindowBounds, WindowOptions, px, size};
use std::{path::Path, time::Duration};
use zuse_ui::*;

async fn pause(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(600))
        .await;
}

async fn capture(window: AnyWindowHandle, cx: &mut AsyncApp, path: &Path) -> anyhow::Result<()> {
    window.update(cx, |_, w, _| w.refresh())?;
    pause(cx).await;
    let status = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/private-secrets-fixture/capture.py"
        ))
        .arg(std::process::id().to_string())
        .arg(path)
        .status()?;
    anyhow::ensure!(status.success(), "fixture window capture failed");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("usage: zapper-create-fixture <dir>"))?,
    );
    std::fs::create_dir_all(&out)?;
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let temp = tempfile::tempdir()?;
    let data = temp.path().to_path_buf();
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_tokio::init(cx);
            gpui_base::init(cx);
            let settings = settings::UiSettings::default();
            settings.save(&data).unwrap();
            settings::init(settings.clone(), data.clone(), cx);
            let fonts = typography::register_fonts(cx);
            typography::init(
                settings.ui_font_family.clone(),
                settings.ui_font_size,
                settings.terminal_font_family.clone(),
                settings.terminal_font_size,
                settings.code_font_family.clone(),
                settings.code_font_size,
                fonts,
                cx,
            );
            theme_library::init(data.clone(), cx);
            appearance::init(
                appearance::AppearanceMode::Dark,
                settings.theme_selection,
                settings.accent,
                settings.surface,
                cx,
            );
            history::init(
                settings.git_history_columns,
                settings.git_history_column_widths,
                settings.git_history_column_order,
                settings.git_history_author_display,
                cx,
            );
            composer::init(cx, settings.composer_send_behavior);
            terminal::panel::init(cx);
            app_menus::init(cx);
            let state = cx.new(|_| {
                let mut s = state::AppState::new();
                s.connection = zuse_protocol::view::ConnectionStatus::Ready;
                s.workspace_scope = Some(zuse_protocol::WorkspaceScope::Local);
                s.local_device_id = Some("local".into());
                s.devices = serde_json::from_value(serde_json::json!([
                    {"id":"local","name":"Studio Mac","platform":"macos","lastSeenAt":null},
                    {"id":"remote","name":"Build server","platform":"linux","lastSeenAt":null}
                ]))
                .unwrap();
                s.auto_selected = true;
                s.chats_synced = true;
                s.spaces_synced = true;
                s.selected_space = Some("web".into());
                let space = |id: &str, device: &str, path: &str| {
                    serde_json::from_value(serde_json::json!({
                        "id": id, "deviceId": device, "path": path,
                        "createdAt": "2026-09-08T00:00:00Z"
                    }))
                    .unwrap()
                };
                s.spaces = vec![
                    space("web", "local", "/Users/sam/code/acme-web"),
                    space("api", "local", "/Users/sam/code/acme-api"),
                    space("docs", "local", "/Users/sam/code/acme-docs"),
                    space("mobile", "local", "/Users/sam/code/acme-mobile"),
                    space("infra", "remote", "/srv/repos/infra"),
                    space("billing", "remote", "/srv/repos/billing-worker"),
                ];
                s
            });
            let boot = EngineBootConfig {
                target: None,
                data_dir: data,
                ipc_port: 0,
                edge_url: String::new(),
                edge_token: None,
                org_id: None,
                workos_client_id: None,
                default_harness: HarnessId::ClaudeCode,
            };
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            gpui::point(px(12.), px(30.)),
                            size(px(1100.), px(900.)),
                        ))),
                        titlebar: Some(gpui::TitlebarOptions {
                            title: None,
                            appears_transparent: true,
                            traffic_light_position: Some(gpui::point(px(14.), px(14.))),
                        }),
                        app_owns_titlebar_drag: true,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx)),
                )
                .unwrap();
            let handle: AnyWindowHandle = window.into();
            cx.spawn(async move |cx| {
                pause(cx).await;
                window
                    .update(cx, |shell, _, cx| shell.fixture_open_new_zapper(cx))
                    .unwrap();
                pause(cx).await;
                capture(handle, cx, &out.join("create.png")).await.unwrap();
                window
                    .update(cx, |shell, _, cx| shell.fixture_scroll_zapper_page(cx))
                    .unwrap();
                pause(cx).await;
                capture(handle, cx, &out.join("create-end.png"))
                    .await
                    .unwrap();
                for (query, file) in [("acme", "search.png"), ("zzz", "no-match.png")] {
                    let searched = window
                        .update(cx, |shell, _, cx| {
                            shell.fixture_search_repositories(query, cx)
                        })
                        .unwrap();
                    if !searched {
                        break;
                    }
                    pause(cx).await;
                    window
                        .update(cx, |shell, _, cx| shell.fixture_scroll_zapper_page(cx))
                        .unwrap();
                    pause(cx).await;
                    capture(handle, cx, &out.join(file)).await.unwrap();
                }
                let opened = window
                    .update(cx, |shell, _, cx| {
                        shell.fixture_search_repositories("", cx) && shell.fixture_open_advanced(cx)
                    })
                    .unwrap();
                if opened {
                    pause(cx).await;
                    window
                        .update(cx, |shell, _, cx| shell.fixture_scroll_zapper_page(cx))
                        .unwrap();
                    pause(cx).await;
                    capture(handle, cx, &out.join("advanced.png"))
                        .await
                        .unwrap();
                }
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
            state.update(cx, |_, cx| cx.notify());
            cx.activate(true);
        });
    Ok(())
}
