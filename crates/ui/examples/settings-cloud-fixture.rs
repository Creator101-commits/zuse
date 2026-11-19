//! Settings review fixture: the real Shell over a scripted engine with
//! synthetic Cloud, billing and usage data. Nothing here is a live account.
//! Run under Xvfb with `ZUSE_OPEN_ROUTE=settings/<section>`, then capture.
//! `FIXTURE_PLAN` picks the plan state (cloud, trial, overage, max, hold,
//! trial-hold, none) and `FIXTURE_THEME=light` the light theme.
//! `FIXTURE_FRESH=1` answers
//! as a just-subscribed account: no GitHub, repositories or agent accounts,
//! and the standard machine. `FIXTURE_ONBOARDING=<step>` opens first-run
//! onboarding at `welcome`, `repositories`, `setup`, `agents` or `start`.
//! `FIXTURE_NO_REPOS=1` keeps GitHub connected but adds no repositories.
use chrono::{Duration, Utc};
use gpui::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use serde_json::{Value, json};
use std::sync::Arc;
use zuse_protocol::methods;
use zuse_protocol::usage::{
    CostStatus, Group, Overview, SessionsPage, SourceStatus, Summary, TokenCounts,
};
use zuse_rpc::{RpcError, RpcReply, RpcService};
use zuse_ui::*;

const OWNER: &str = "fixture-owner";
const DAY: i64 = 86_400_000;

fn now() -> i64 {
    Utc::now().timestamp_millis()
}
fn bound(data: Value) -> Result<RpcReply, RpcError> {
    RpcReply::value(&json!({"accountId": OWNER, "organizationId": null, "data": data}))
}
fn iso(days_ago: i64) -> String {
    (Utc::now() - Duration::days(days_ago)).to_rfc3339()
}

fn repositories() -> Value {
    let repos = [
        ("swarajbachu/zuse", 1, true),
        ("swarajbachu/spacekey", 63, false),
        ("swarajbachu/framezero", 21, false),
        ("swarajbachu/userbubble", 30, true),
        ("swarajbachu/www", 49, false),
        ("opcodehq/zuse", 0, true),
        ("opcodehq/zkit", 2, true),
        ("opcodehq/web", 8, true),
        ("opcodehq/computer-use", 9, false),
        ("opcodehq/postkit", 35, true),
    ];
    json!(
        repos
            .iter()
            .map(|(name, ago, private)| json!({
                "nameWithOwner": name,
                "description": null,
                "sshUrl": format!("git@github.com:{name}.git"),
                "httpsUrl": format!("https://github.com/{name}"),
                "isPrivate": private,
                "defaultBranch": "main",
                "updatedAt": iso(*ago),
            }))
            .collect::<Vec<_>>()
    )
}

fn projects() -> Value {
    let rows = [
        ("p-zuse", "opcodehq/zuse", "private"),
        ("p-web", "opcodehq/web", "private"),
        ("p-space", "swarajbachu/spacekey", "public"),
    ];
    json!({"projects": rows.iter().map(|(id, name, vis)| json!({
        "projectId": id, "repositoryIdentity": format!("github.com/{name}"),
        "repositoryUrl": format!("https://github.com/{name}"), "displayName": name,
        "defaultBranch": "main", "visibility": vis, "state": "ready",
        "activeBuilds": {}, "latestBuilds": {}, "createdAt": now() - 9 * DAY, "updatedAt": now() - DAY,
    })).collect::<Vec<_>>()})
}

/// Computers that ran this period, named by their chats.
const MACHINES: [(&str, &str, &str); 5] = [
    ("ws-login", "chat-login", "zuse/fix-login-redirect"),
    ("ws-billing", "chat-billing", "zuse/usage-breakdown"),
    ("ws-guide", "chat-guide", "zuse/cloud-setup-guide"),
    ("ws-docs", "chat-docs", "web/pricing-copy"),
    ("ws-flaky", "chat-flaky", "zuse/flaky-sync-test"),
];
const TITLES: [&str; 5] = [
    "Fix the login redirect loop",
    "Break usage down by computer",
    "Guided Cloud setup",
    "Rewrite the pricing page copy",
    "Find the flaky sync test",
];

/// Which plan state the fixture shows (`FIXTURE_PLAN`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlanState {
    /// Cloud, a third of the way through its machine minutes.
    Cloud,
    /// The $1 trial, part used.
    Trial,
    /// Cloud past its allowance, paying as it goes under the cap.
    Overage,
    /// Cloud Max with a large computer.
    Max,
    /// Cloud with the pay-as-you-go cap reached: computers paused.
    Hold,
    /// The trial used up: computers paused.
    TrialHold,
    /// Not subscribed: plans and the $1 trial.
    None,
}
impl PlanState {
    fn from_env() -> Self {
        match std::env::var("FIXTURE_PLAN").as_deref() {
            Ok("trial") => Self::Trial,
            Ok("overage") => Self::Overage,
            Ok("max") => Self::Max,
            Ok("hold") => Self::Hold,
            Ok("trial-hold") => Self::TrialHold,
            Ok("none") => Self::None,
            _ => Self::Cloud,
        }
    }
    fn trial(self) -> bool {
        matches!(self, Self::Trial | Self::TrialHold)
    }
}

fn usage_items(start: i64, plan: PlanState) -> Value {
    let mut items = vec![];
    let mut n = 0;
    // Busier days push the overage states past the allowance.
    let scale = match plan {
        PlanState::Overage | PlanState::Hold => 16,
        PlanState::Max => 6,
        PlanState::Trial | PlanState::TrialHold => 3,
        _ => 4,
    };
    let days = if plan.trial() { 4 } else { 6 };
    for day in 0..days {
        for (i, (ws, _, _)) in MACHINES.iter().enumerate() {
            if (day + i as i64) % 3 == 2 {
                continue;
            }
            let awake = (12 + ((day * 7 + i as i64 * 11) % 40)) * scale;
            let large = plan == PlanState::Max && i == 1;
            let rate = if large { 8 } else { 4 };
            let begin = start + day * DAY + (i as i64 + 9) * 3_600_000;
            n += 1;
            items.push(json!({"entryId": format!("e{n:04}"), "resourceKind": "workspace", "resourceId": ws,
                "provider": "cloud", "startedAt": begin, "endedAt": begin + awake * 60_000,
                "vcpuCount": if large { 8 } else { 4 }, "memoryMib": if large { 32_768 } else { 16_384 },
                "providerCostMicros": awake * rate * 595, "status": "confirmed",
                "awakeMs": awake * 60_000, "machineRate": rate, "machineMinutes": awake * rate}));
        }
    }
    for (k, day) in [0i64, 3].iter().enumerate() {
        let begin = start + day * DAY + 8 * 3_600_000;
        items.push(json!({"entryId": format!("b{k}"), "resourceKind": "build", "resourceId": format!("build-{k}"),
            "provider": "cloud", "startedAt": begin, "endedAt": begin + 9 * 60_000, "vcpuCount": 4, "memoryMib": 16_384,
            "providerCostMicros": 21_420, "status": "confirmed", "awakeMs": 9 * 60_000, "machineRate": 4, "machineMinutes": 36}));
    }
    items.push(json!({"entryId": "s0", "resourceKind": "other", "usageKind": "snapshot-storage", "resourceId": "image",
        "provider": "cloud", "startedAt": start, "endedAt": now(), "vcpuCount": 0, "memoryMib": 0,
        "providerCostMicros": 310_000, "status": "provisional", "machineMinutes": 0}));
    let begin = now() - 25 * 60_000;
    items.push(json!({"entryId": "live", "resourceKind": "workspace", "resourceId": "ws-guide", "provider": "cloud",
        "startedAt": begin, "endedAt": now(), "vcpuCount": 4, "memoryMib": 16_384, "providerCostMicros": 59_500,
        "status": "provisional", "awakeMs": 25 * 60_000, "machineRate": 4, "machineMinutes": 100}));
    json!({"items": items})
}

/// Machine minutes the synthetic usage adds up to, so the plan agrees with it.
fn used_minutes(start: i64, plan: PlanState) -> i64 {
    usage_items(start, plan)["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["machineMinutes"].as_i64())
        .sum()
}

fn summary(start: i64, plan: PlanState) -> Value {
    let end = start + if plan.trial() { 7 * DAY } else { 30 * DAY };
    let used = used_minutes(start, plan);
    let (id, name, price, included, max) = match plan {
        PlanState::Trial | PlanState::TrialHold => {
            ("cloud-trial", "Cloud trial", 1_000_000, 8_400, false)
        }
        PlanState::Max => ("cloud-max", "Cloud Max", 50_000_000, 84_000, true),
        _ => ("cloud", "Cloud", 20_000_000, 33_600, false),
    };
    let cap = 25_000_000i64;
    let used = match plan {
        PlanState::TrialHold => used.max(included),
        // The cap reached: $25 of extra minutes past the allowance.
        PlanState::Hold => included + 42_000,
        _ => used,
    };
    let extra = (used - included).max(0);
    let charge = zuse_protocol::cloud_billing::plans::machine_minutes_micros(extra);
    let (hold, reason) = match plan {
        PlanState::Hold => (true, Some("cap-reached")),
        PlanState::TrialHold => (true, Some("trial-allowance-used")),
        _ => (false, None),
    };
    let mut plan_usage = json!({"planId": id, "planName": name, "priceMicros": price, "trial": plan.trial(),
        "renewsAt": end, "includedMachineMinutes": included, "usedMachineMinutes": used.min(included) + extra,
        "remainingMachineMinutes": (included - used).max(0), "extraMachineMinutes": if plan.trial() { 0 } else { extra },
        "extraChargeMicros": if plan.trial() { 0 } else { charge }, "overageCapMicros": cap, "payAsYouGo": !plan.trial(),
        "usageHold": hold, "defaultMachineRate": 4, "maxVcpus": if max { 8 } else { 4 },
        "maxMemoryMib": if max { 32_768 } else { 16_384 }});
    if let Some(reason) = reason {
        plan_usage["holdReason"] = json!(reason);
    }
    if plan.trial() {
        plan_usage["trialEndsAt"] = json!(end);
    }
    json!({"currency": "USD", "status": "active", "periodStart": start, "periodEnd": end,
        "basePriceMicros": price, "includedProviderCostMicros": price, "providerCostMicros": 8_420_000,
        "storageCostMicros": 310_000, "includedUsedMicros": 8_420_000, "includedRemainingMicros": 11_580_000,
        "overageProviderCostMicros": 0, "overageChargeMicros": charge, "overageCapMicros": cap,
        "markupBasisPoints": 0, "currentInvoiceEstimateMicros": price + charge, "usageProvisional": false,
        "plan": plan_usage})
}

fn entitlements(start: i64, plan: PlanState) -> Value {
    let catalog = zuse_protocol::cloud_billing::plans::catalog(
        plan == PlanState::None,
        &|_| true,
        zuse_protocol::cloud_billing::plans::ENTERPRISE_CONTACT_FALLBACK,
    );
    let offer = match plan {
        PlanState::Max => "cloud-pro-v1",
        _ => "cloud-starter-v1",
    };
    let entitlements = if plan == PlanState::None {
        json!([])
    } else {
        json!([{"offerId": offer, "status": "active", "paidThrough": start + 30 * DAY}])
    };
    json!({"entitlements": entitlements, "catalog": catalog})
}

fn tokens(scale: u64) -> Summary {
    Summary {
        tokens: TokenCounts {
            input_tokens: 41_000 * scale,
            output_tokens: 6_200 * scale,
            cache_read_tokens: 310_000 * scale,
            cache_creation_tokens: 9_000 * scale,
            reasoning_tokens: 2_100 * scale,
        },
        cost_usd: Some(0.42 * scale as f64),
        cost_status: CostStatus::Known,
        record_count: 12 * scale as usize,
        possible_duplicate_count: 0,
    }
}
fn group(key: &str, label: &str, days_ago: i64, scale: u64, source: &str) -> Group {
    let at = Utc::now() - Duration::days(days_ago);
    Group {
        key: key.into(),
        label: label.into(),
        started_at: Some(at),
        ended_at: Some(at + Duration::hours(1)),
        source_ids: vec![source.into()],
        summary: tokens(scale),
    }
}
fn overview() -> Overview {
    let groups = (0..30)
        .rev()
        .map(|d| group(&format!("d{d}"), "", d, 1 + ((d * 7) % 9) as u64, "codex"))
        .collect();
    Overview {
        bucket: "day".into(),
        generated_at: Utc::now(),
        summary: tokens(140),
        session_count: 38,
        previous_summary: Some(tokens(96)),
        previous_session_count: Some(27),
        groups,
        by_source: vec![
            group("codex", "Codex", 0, 92, "codex"),
            group("claude", "Claude Code", 0, 48, "claude"),
        ],
        by_model: vec![
            group("gpt", "GPT-5.5", 0, 81, "codex"),
            group("sonnet", "Claude Sonnet 5.5", 0, 48, "claude"),
            group("mini", "GPT-5.5 mini", 0, 11, "codex"),
        ],
        by_project: vec![
            group("zuse", "zuse", 0, 104, "codex"),
            group("web", "web", 0, 36, "claude"),
        ],
        previous_by_source: vec![],
        previous_by_model: vec![],
        previous_by_project: vec![],
        sources: vec![
            SourceStatus {
                id: "codex".into(),
                label: "Codex".into(),
                detected: true,
                record_count: 1104,
                paths: vec![],
                warning: None,
            },
            SourceStatus {
                id: "claude".into(),
                label: "Claude Code".into(),
                detected: true,
                record_count: 576,
                paths: vec![],
                warning: None,
            },
        ],
    }
}
fn sessions() -> SessionsPage {
    let rows: Vec<Group> = TITLES
        .iter()
        .enumerate()
        .map(|(i, t)| {
            group(
                &format!("s{i}"),
                t,
                i as i64,
                30 - 5 * i as u64,
                if i % 2 == 0 { "codex" } else { "claude" },
            )
        })
        .collect();
    SessionsPage {
        total: rows.len(),
        rows,
        next_offset: None,
    }
}

struct Controller {
    start: i64,
    plan: PlanState,
    /// A just-subscribed account with nothing set up yet.
    fresh: bool,
    /// GitHub connected, no repositories added yet.
    no_repos: bool,
}
#[async_trait::async_trait]
impl RpcService for Controller {
    async fn handle(&self, method: &str, _params: Value) -> Result<RpcReply, RpcError> {
        if self.no_repos && method == methods::CLOUD_PROJECTS {
            return bound(json!({"projects": []}));
        }
        if self.fresh {
            match method {
                methods::CLOUD_PROJECTS => return bound(json!({"projects": []})),
                methods::CLOUD_GITHUB => {
                    return bound(json!({"configured": true, "canManageInstallations": true,
                        "user": null, "installations": [], "repositories": []}));
                }
                methods::CLOUD_AGENT_CONNECTIONS => {
                    return bound(json!({"connections": [], "supportsDeviceReconnect": true}));
                }
                methods::CLOUD_MACHINE => {
                    return bound(
                        json!({"revision": 0, "createdAt": now(), "imageState": "none",
                        "imageError": null, "recipe": {"version": 1, "setupScript": "", "skills": [],
                        "instructions": "", "files": [], "setupTimeoutSeconds": 900}}),
                    );
                }
                _ => {}
            }
        }
        match method {
            methods::LOCAL_DEVICE => RpcReply::value(
                &json!({"deviceId":"fixture-local","workspaceScope":"local","capabilities":[]}),
            ),
            methods::CLOUD_PROJECTS => bound(projects()),
            methods::CLOUD_GITHUB => bound(json!({
                "configured": true, "canManageInstallations": true,
                "user": {"login": "swarajbachu", "name": "Swaraj", "email": "you@example.com"},
                "installations": [
                    {"installationId": 101, "accountLogin": "swarajbachu", "accountType": "User", "repositorySelection": "all", "suspended": false},
                    {"installationId": 202, "accountLogin": "opcodehq", "accountType": "Organization", "repositorySelection": "selected", "suspended": false}
                ],
                "repositories": repositories(),
            })),
            methods::CLOUD_AGENT_CONNECTIONS => bound(json!({"connections": [
                {"connectionId": "codex", "agent": "codex", "method": "device-code", "state": "connected", "accountLabel": "you@example.com", "expiresAt": null, "version": 1, "errorCode": null},
                {"connectionId": "claude", "agent": "claude", "method": "setup-token", "state": "connected", "accountLabel": "Claude Max", "expiresAt": null, "version": 1, "errorCode": null}
            ], "supportsDeviceReconnect": true})),
            methods::CLOUD_MACHINE => bound(
                json!({"revision": 3, "createdAt": now() - 2 * DAY, "imageState": "ready", "imageError": null,
                "recipe": {"version": 1, "setupScript": "curl -fsSL https://bun.sh/install | bash\nnpm i -g pnpm@9 turbo\napt-get install -y ripgrep jq",
                    "skills": [{"name": "release-notes", "content": "Write release notes from merged PRs."}],
                    "instructions": "Prefer small commits. Run the tests that cover what you changed before pushing.",
                    "files": [], "setupTimeoutSeconds": 900}}),
            ),
            methods::CLOUD_PROJECT_ENVIRONMENT => bound(
                json!({"projectId": _params["projectId"], "revision": 2, "source": "project", "createdAt": now() - DAY,
                "recipe": {"version": 1, "scripts": {"initialize": "pnpm install", "refresh": "pnpm install --frozen-lockfile"},
                    "startup": [{"name": "web", "command": "pnpm dev --port 3000", "port": 3000, "timeoutSeconds": 60}],
                    "environment": {"NODE_ENV": "development", "API_URL": "http://localhost:3000"}, "secretBindings": ["sec-1"], "setupTimeoutSeconds": 900}}),
            ),
            methods::CLOUD_PROJECT_SECRETS => bound(json!({"secrets": [
                {"secretId": "sec-1", "name": "DATABASE_URL", "revision": 1, "updatedAt": now() - 3 * DAY},
                {"secretId": "sec-2", "name": "STRIPE_TEST_KEY", "revision": 2, "updatedAt": now() - DAY}
            ]})),
            methods::CLOUD_BILLING_SUMMARY | methods::CLOUD_BILLING_USAGE
                if self.plan == PlanState::None =>
            {
                Err(RpcError::Failed(
                    "An active cloud billing period is required.".into(),
                ))
            }
            methods::CLOUD_BILLING_SUMMARY => bound(summary(self.start, self.plan)),
            methods::CLOUD_BILLING_USAGE => bound(usage_items(self.start, self.plan)),
            methods::CLOUD_BILLING_ENTITLEMENTS => bound(entitlements(self.start, self.plan)),
            methods::CLOUD_PREPAID_BALANCE => bound(
                json!({"available": true, "creditCents": 1250, "debitCents": 0, "currency": "USD"}),
            ),
            methods::CLOUD_WORKSPACES => {
                RpcReply::value(&json!({"accountId": OWNER, "organizationId": null,
                "workspaces": MACHINES.iter().map(|(ws, chat, branch)| json!({"workspaceId": ws, "chatId": chat, "branch": branch, "state": "ready"})).collect::<Vec<_>>()}))
            }
            zuse_protocol::usage::OVERVIEW => RpcReply::value(&overview()),
            zuse_protocol::usage::SESSIONS => RpcReply::value(&sessions()),
            methods::LIST_AGENT_ACCOUNTS => {
                RpcReply::value(&zuse_protocol::AgentAccountsSnapshot::default())
            }
            _ => Err(RpcError::Capability(format!("fixture.{method}"))),
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter("error").init();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let _enter = runtime.enter();
    let plan = PlanState::from_env();
    let start = now() - if plan.trial() { 3 } else { 5 } * DAY - 3 * 3_600_000;
    let fresh = std::env::var("FIXTURE_FRESH").is_ok();
    let client = zuse_rpc::memory_client(Arc::new(Controller {
        start,
        plan,
        fresh,
        no_repos: std::env::var("FIXTURE_NO_REPOS").is_ok(),
    }));
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().to_path_buf();
    gpui_platform::application().with_assets(icons::Assets).run(move |cx| {
        gpui_tokio::init(cx);
        gpui_base::init(cx);
        let settings = settings::UiSettings::default();
        settings::init(settings.clone(), data.clone(), cx);
        let fonts = typography::register_fonts(cx);
        typography::init(settings.ui_font_family.clone(), settings.ui_font_size,
            settings.terminal_font_family.clone(), settings.terminal_font_size,
            settings.code_font_family.clone(), settings.code_font_size, fonts, cx);
        theme_library::init(data.clone(), cx);
        let mode = if std::env::var("FIXTURE_THEME").as_deref() == Ok("light") {
            appearance::AppearanceMode::Light
        } else {
            appearance::AppearanceMode::Dark
        };
        appearance::init(mode, settings.theme_selection,
            settings.accent, settings.surface, cx);
        history::init(settings.git_history_columns, settings.git_history_column_widths,
            settings.git_history_column_order, settings.git_history_author_display, cx);
        composer::init(cx, settings.composer_send_behavior);
        terminal::panel::init(cx);
        app_menus::init(cx);
        let state = cx.new(|_| {
            let mut s = state::AppState::new();
            s.connection = zuse_protocol::view::ConnectionStatus::Ready;
            s.workspace_scope = Some(zuse_protocol::WorkspaceScope::Local);
            s.local_device_id = Some("fixture-local".into());
            s.devices = vec![serde_json::from_value(json!({"id":"fixture-local","name":"Studio Mac","platform":"macos","lastSeenAt":null})).unwrap()];
            s.auto_selected = true;
            s.spaces_synced = true;
            s.chats_synced = true;
            s.apply_cloud_account(json!({"user":{"id": OWNER,"email":"you@example.com"},"orgId":null}));
            s.fixture_cloud_subscription(plan != PlanState::None);
            s.chats = MACHINES.iter().zip(TITLES).map(|((_, chat, branch), title)| serde_json::from_value(json!({
                "id": chat, "deviceId": "fixture-local", "title": title, "branch": branch, "archived": false,
                "createdAt": "2026-11-02T12:00:00Z"})).unwrap()).collect();
            s.fixture_cloud_sidebar_catalog(client, &json!({}), &json!({"accountId": OWNER, "organizationId": null, "workspaces": []}));
            s
        });
        let boot = EngineBootConfig { target: None, data_dir: data, ipc_port: 0, edge_url: String::new(),
            edge_token: None, org_id: None, workos_client_id: None, default_harness: HarnessId::Codex };
        let width: f32 = std::env::var("FIXTURE_WIDTH").ok().and_then(|v| v.parse().ok()).unwrap_or(1280.);
        let height: f32 = std::env::var("FIXTURE_HEIGHT").ok().and_then(|v| v.parse().ok()).unwrap_or(1500.);
        let window = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(width), px(height))))),
            ..Default::default()
        }, |_, cx| cx.new(|cx| shell::Shell::new(state.clone(), boot, cx))).unwrap();
        state.update(cx, |_, cx| cx.notify());
        cx.activate(true);
        let out = std::env::var("FIXTURE_OUT").ok();
        let compute = std::env::var("FIXTURE_COMPUTE").is_ok();
        let onboarding = std::env::var("FIXTURE_ONBOARDING").ok();
        cx.spawn(async move |cx| {
            let delay = std::env::var("FIXTURE_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2500);
            cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
            if let Some(step) = onboarding {
                window
                    .update(cx, |shell, window, cx| {
                        shell.open_cloud_onboarding(Some(step.as_str()), window, cx)
                    })
                    .unwrap();
                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
            }
            if compute {
                window.update(cx, |shell, _, cx| shell.fixture_usage_compute(cx)).unwrap();
                cx.background_executor().timer(std::time::Duration::from_millis(2000)).await;
            }
            let Some(out) = out else { return };
            let any: gpui::AnyWindowHandle = window.into();
            any.update(cx, |_, window, cx| {
                window.draw(cx).clear();
                window.present_if_needed();
                match window.render_to_image() {
                    Ok(image) => image.save(&out).expect("save"),
                    Err(_) => {
                        let status = std::process::Command::new("import")
                            .args(["-window", "root", &out])
                            .status()
                            .expect("import");
                        assert!(status.success(), "X11 capture failed");
                    }
                }
            }).unwrap();
            std::process::exit(0);
        }).detach();
    });
}
