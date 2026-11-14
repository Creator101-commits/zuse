// Saved PostHog query definitions. No credentials or production requests here.
export const property = (key, value) => ({ key, value: Array.isArray(value) ? value : [value], operator: "exact", type: "event" });
const series = (event, { surface = "engine", math = "total", properties = [], ...rest } = {}) => ({
  kind: "EventsNode", event, name: event, math,
  properties: [property("analytics_schema_version", 2), ...(surface ? [property("surface", surface)] : []), ...properties],
  ...rest,
});
const trend = (event, { breakdown, interval = "day", ...options } = {}) => ({
  kind: "TrendsQuery", dateRange: { date_from: "-30d" }, interval,
  series: [series(event, options)],
  trendsFilter: { display: breakdown ? "ActionsBar" : "ActionsLineGraph" },
  ...(breakdown ? { breakdownFilter: { breakdown, breakdown_type: "event" } } : {}),
});
const tile = (name, description, source) => ({ name, description, query: { kind: "InsightVizNode", source } });
const engine = (name, event, description, options = {}) => tile(`Zuse — ${name}`, description, trend(event, options));
const clientSurfaces = ["desktop", "web", "ios", "android"];
export const analyticsDashboards = [
  {
    name: "Zuse — Chats and activation",
    description: "New conversations, client navigation opens, submitted messages and activation. Opens are navigation visits, not unique conversations. No conversation identifiers are collected.",
    tiles: [
      engine("Chats created", "chat created", "New persisted conversation rows; repeated creation of an existing id is excluded."),
      engine("Chat creators", "chat created", "Unique pseudonymous identities creating chats per day.", { math: "dau" }),
      engine("Chats opened", "chat opened", "Total navigation into a conversation, including returning visits; transcript reconnects are excluded.", { surface: clientSurfaces }),
      engine("Chat opens by client", "chat opened", "Desktop, web, iOS and Android navigation visits.", { surface: clientSurfaces, breakdown: "surface" }),
      engine("Messages submitted", "message submitted", "Persisted run/steer commands; retries with the same message id in the command ledger are excluded."),
      engine("Messages by kind", "message submitted", "Run versus steer submissions.", { breakdown: "submission_kind" }),
      engine("Archived and restored chats", "chat archived", "Successful archive transitions, not archive RPC attempts."),
      engine("Chats restored", "chat restored", "Successful unarchive transitions."),
      tile("Zuse — Activation funnel", "Same engine identity creating a chat, submitting a message and receiving a completed turn within one day. No cross-device browser/install linkage is implied.", {
        kind: "FunnelsQuery", dateRange: { date_from: "-30d" },
        series: ["chat created", "message submitted", "turn completed"].map((event, order) => ({ ...series(event), order })),
        funnelsFilter: { funnelVizType: "steps", funnelOrderType: "ordered", funnelWindowInterval: 1, funnelWindowIntervalUnit: "day" },
      }),
    ],
  },
  {
    name: "Zuse — Models and settings",
    description: "Actual vendor model IDs and selected settings. Custom/local identifiers are grouped as custom; default is a provider-selected model until the provider reports its actual model.",
    tiles: [
      engine("Messages by model", "message submitted", "Declared model choice at submission.", { breakdown: "model" }),
      engine("Messages by provider", "message submitted", "Built-in harness kind; custom ACP instances share acp.", { breakdown: "provider" }),
      engine("Completed turns by selected model", "turn completed", "Execution outcomes by declared model.", { breakdown: "model" }),
      engine("Completed turns by actual model", "turn completed", "Last main-agent model reported by the provider. Missing reports stay absent; check mixed_models before comparisons.", { breakdown: "actual_model" }),
      engine("Reasoning choices", "message submitted", "Requested reasoning effort; default means no explicit choice.", { breakdown: "reasoning_effort" }),
      engine("Access settings", "message submitted", "Approval-required workspace writes, automatic review, or full access.", { breakdown: "permission_mode" }),
      engine("Plan mode", "message submitted", "Requested plan-mode setting.", { breakdown: "plan_mode" }),
      engine("Model changes", "model changed", "Successful changes of model or provider; unchanged configuration writes are excluded.", { breakdown: "model" }),
      engine("Settings changes", "chat settings changed", "Changes to settings while provider/model stay the same.", { breakdown: "reasoning_effort" }),
    ],
  },
  {
    name: "Zuse — Execution and reliability",
    description: "Main-agent execution and observed provider usage. Tokens are provider-reported snapshots or native request sums, not verified billing or complete nested-agent totals. Costs are not inferred.",
    tiles: [
      tile("Zuse — Turn outcomes", "One event per observed completion, failure or interruption; continuations may start without a new user submission.", {
        ...trend("turn completed"), series: ["turn completed", "turn failed", "turn interrupted"].map(event => series(event)),
      }),
      engine("Median turn duration by model", "turn completed", "Main-agent elapsed time, including tool and approval waits.", { breakdown: "model", math: "median", math_property: "duration_ms" }),
      engine("P95 turn duration", "turn completed", "95th percentile of completed-turn elapsed time.", { math: "p95", math_property: "duration_ms" }),
      engine("P95 first text latency", "turn completed", "Time until the first nonempty main-agent text delta; tool-only turns have no measurement.", { math: "p95", math_property: "first_token_ms" }),
      engine("Failures by model", "turn failed", "Failed turn counts; use matching started/completed cohorts when calculating rates.", { breakdown: "model" }),
      engine("Tool calls", "turn completed", "Unique tool IDs counted within each main-agent turn.", { math: "sum", math_property: "tool_count" }),
      engine("Tool failures", "turn completed", "Unique failed tool-result IDs counted within each turn.", { math: "sum", math_property: "tool_failure_count" }),
      tile("Zuse — Tool categories", "Aggregated typed tool categories, never raw tool names, arguments or results.", {
        ...trend("turn completed"), series: ["shell", "files", "browser", "mcp", "subagent", "other"].map(category => ({
          ...series("turn completed", { math: "sum", math_property: `${category}_tool_count` }), name: category,
        })),
      }),
      engine("Observed input tokens", "turn completed", "Input tokens where a main-agent usage measurement was observed; missing is not zero.", { math: "sum", math_property: "input_tokens", properties: [property("usage_status", "observed")] }),
      engine("Observed output tokens", "turn completed", "Output tokens where a main-agent usage measurement was observed.", { math: "sum", math_property: "output_tokens", properties: [property("usage_status", "observed")] }),
      engine("Observed cache read tokens", "turn completed", "Optional native request-accounting cache reads; absent measurements are not zero.", { math: "sum", math_property: "cache_read_tokens" }),
      engine("Observed cache write tokens", "turn completed", "Optional native cache creation measurements.", { math: "sum", math_property: "cache_creation_tokens" }),
      engine("Observed reasoning tokens", "turn completed", "Optional native reasoning-token measurements.", { math: "sum", math_property: "reasoning_tokens" }),
      engine("Native request failures", "turn completed", "Observed failed native attempts, including retries.", { math: "sum", math_property: "request_failure_count" }),
      engine("Usage coverage", "turn completed", "Observed versus unknown usage; inspect this before treating token totals as representative.", { breakdown: "usage_status" }),
      engine("Subagent activity", "turn completed", "Distinct observed subagent spawn references, excluding their token usage from parent totals.", { math: "sum", math_property: "subagent_count" }),
      engine("Continuation turns", "turn completed", "Whether execution resumed from provider-initiated continuation work.", { breakdown: "continuation" }),
    ],
  },
  {
    name: "Zuse — Engagement and environment",
    description: "Observed account/install activity, client engagement and feature use. Unique identities are not exact people. Daily uniques must not be summed into a monthly unique count.",
    tiles: [
      engine("Daily active submitters", "message submitted", "Unique identities submitting commands per day. The current protocol does not distinguish all human versus agent-authored commands.", { math: "dau" }),
      engine("Weekly active submitters", "message submitted", "Unique identities per weekly bucket.", { math: "dau", interval: "week" }),
      tile("Zuse — Active installations", "Distinct stable installation IDs over the last 30 days, independently of account sign-in.", {
        kind: "HogQLQuery",
        query: "SELECT uniqExact(properties.install_id) AS active_installations FROM events WHERE event = 'message submitted' AND properties.analytics_schema_version = 2 AND properties.surface = 'engine' AND isNotNull(properties.install_id) AND timestamp >= now() - INTERVAL 30 DAY",
      }),
      engine("Operating systems", "message submitted", "Execution-host operating systems; this is not the connected client's OS.", { math: "dau", breakdown: "os" }),
      engine("Architectures", "message submitted", "Execution-host CPU architectures.", { math: "dau", breakdown: "architecture" }),
      engine("App versions", "message submitted", "Engine package version.", { math: "dau", breakdown: "app_version" }),
      engine("Feature use", "feature used", "Observed RPC attempts, unlike durable lifecycle events; forwarded calls can appear on two hosts.", { breakdown: "feature" }),
      engine("Feature outcomes", "feature used", "Success/failure of observed feature RPCs.", { breakdown: "outcome" }),
      engine("Browser active seconds", "app active interval", "Focused, visible interaction time; idle after 60 seconds.", { surface: "web", math: "sum", math_property: "active_seconds" }),
      engine("Browser screens", "screen viewed", "Screen categories, excluding paths and entity identifiers.", { surface: "web", breakdown: "screen" }),
    ],
  },
];

const terminalEvents = ["turn completed", "turn failed", "turn interrupted"];
const terminalMetric = (name, description, options = {}) => tile(`Zuse — ${name}`, description, {
  ...trend("turn completed", options),
  series: terminalEvents.map(event => series(event, options)),
});

analyticsDashboards[1].tiles.push(
  engine("Selected service tiers", "message submitted", "Explicit supported Codex tier or Claude fast-mode selection; unknown/unsupported settings remain absent.", { breakdown: "service_tier" }),
);
analyticsDashboards.push({
  name: "Zuse — Usage and coverage",
  description: "Observed root-turn measurements across completed, failed and interrupted turns. Partial measurements are lower bounds; absent usage is unknown. These are not request counts, complete child-agent totals, or bills.",
  tiles: [
    terminalMetric("All-outcome input tokens", "Reported root input tokens, split by terminal outcome; partial interrupted/failed measurements are retained.", { math: "sum", math_property: "input_tokens" }),
    terminalMetric("All-outcome output tokens", "Reported root output tokens across every terminal outcome; not estimated from character counts.", { math: "sum", math_property: "output_tokens" }),
    terminalMetric("All-outcome cache read tokens", "Reported cache reads where available. Unknown is absent, not zero.", { math: "sum", math_property: "cache_read_tokens" }),
    terminalMetric("All-outcome cache write tokens", "Reported cache creation where available; no inferred pricing.", { math: "sum", math_property: "cache_creation_tokens" }),
    terminalMetric("All-outcome reasoning tokens", "Reported reasoning tokens, which may be a subset of output; do not add them to output totals.", { math: "sum", math_property: "reasoning_tokens" }),
    terminalMetric("Usage measurement coverage", "New reports distinguish observed completed usage, partial interrupted/failed usage, and unknown. All three outcomes remain in the denominator.", { breakdown: "usage_status", properties: [property("usage_report_version", 1)] }),
    terminalMetric("Root output characters", "Unicode scalar count of streamed root text. No content is sent; this is not a token estimate.", { math: "sum", math_property: "output_character_count" }),
    terminalMetric("Root reasoning characters", "Root reasoning character counts without the reasoning content. Provider redaction can limit observation.", { math: "sum", math_property: "reasoning_character_count" }),
    terminalMetric("Approval and input prompts", "Unique root input-request identifiers per turn; repeated notifications are excluded. Includes questions and approvals.", { math: "sum", math_property: "input_request_count" }),
    terminalMetric("All-outcome tool calls", "Distinct root tool IDs within each turn, including failed/interrupted turns.", { math: "sum", math_property: "tool_count" }),
    terminalMetric("All-outcome tool failures", "Distinct failed root tool result IDs, including failed/interrupted turns.", { math: "sum", math_property: "tool_failure_count" }),
    terminalMetric("Root context occupancy", "Median latest measured root context tokens, excluding nested agents; missing and explicit zero remain distinct.", { math: "median", math_property: "context_tokens" }),
    terminalMetric("Root context capacity", "Median latest reported root context window, excluding nested agents.", { math: "median", math_property: "context_window_tokens" }),
    terminalMetric("All-outcome duration", "P95 observed root-turn elapsed time by terminal outcome, including tool and approval waits.", { math: "p95", math_property: "duration_ms" }),
    terminalMetric("Terminal outcomes by provider", "Completed, failed, and interrupted root-turn cohorts for each provider; includes continuations and child-chat turns.", { breakdown: "provider" }),
  ],
});

const retention = (name, period, totalIntervals) => tile(`Zuse — ${name}`,
  "Pseudonymous identities returning to submit a persisted command. Cohorts start at their first submission within the selected range, not verified first-ever use; human versus automation origin is unknown.", {
    kind: "RetentionQuery", dateRange: { date_from: period === "Week" ? "-90d" : "-30d" },
    properties: [property("analytics_schema_version", 2), property("surface", "engine")],
    retentionFilter: {
      period, totalIntervals, retentionType: "retention_first_time", retentionReference: "total", cumulative: false,
      targetEntity: { id: "message submitted", name: "message submitted", type: "events" },
      returningEntity: { id: "message submitted", name: "message submitted", type: "events" },
    },
  });
const featureMetric = (name, description, feature, options = {}) => engine(name, "feature used", description, {
  ...options, properties: [property("feature", feature), ...(options.properties ?? [])],
});
analyticsDashboards.push({
  name: "Zuse — Retention and feature adoption",
  description: "Returning submitter identities and explicit feature RPCs. RPC success means a successful response, not guaranteed business completion; no-op/refused replies may still succeed. Forwarding/retries can add attempts, so these are not durable outcome counts.",
  tiles: [
    retention("Daily submitter retention", "Day", 14),
    retention("Weekly submitter retention", "Week", 8),
    engine("Monthly active submitters", "message submitted", "Unique submitting identities per calendar-month bucket, not summed daily uniques or exact people.", { math: "dau", interval: "month" }),
    engine("RPC feature adopters", "feature used", "Unique identities using each allowlisted feature; one identity may use several features.", { math: "dau", breakdown: "feature" }),
    featureMetric("Git action breakdown", "Explicit stage/unstage, commits, push/pull/fetch, branch and pull-request creation attempts; no paths, branch names, titles or messages.", "git action", { breakdown: "variant" }),
    featureMetric("Pull request action breakdown", "Explicit merge/auto-merge, ready/draft, close/reopen, edit and reviewer actions; no request IDs or text.", "pull request updated", { breakdown: "variant" }),
    featureMetric("Archive versus restore attempts", "Requested directions, separate from durable archive/restore transition events.", "chat archived", { breakdown: "variant" }),
    engine("Feature failure categories", "feature used", "Typed RPC error category only; raw errors and provider messages are excluded.", { breakdown: "failure_kind", properties: [property("outcome", "failure")] }),
    engine("Feature failure counts", "feature used", "Failed RPC attempts grouped by feature; include matching attempt counts when interpreting rates.", { breakdown: "feature", properties: [property("outcome", "failure")] }),
    engine("Feature P95 latency", "feature used", "RPC response latency by action; queued asynchronous work can finish later.", { math: "p95", math_property: "duration_ms", breakdown: "feature" }),
    featureMetric("Conversation imports", "Import RPC attempts only, with no source titles, identifiers or content.", "conversation imported"),
    featureMetric("Delivery retry use", "Explicit user delivery retries; standing sync probes and reconnects do not count.", "delivery retried"),
  ],
});

const native = (name, event, description, options = {}) => engine(`Desktop ${name}`, event, description, { ...options, surface: "desktop" });
analyticsDashboards.push({
  name: "Zuse — Native desktop",
  description: "Native UI transitions through the connected engine's consent and identity. Focused time is foreground dwell, including reading and idle time; it is not verified human interaction. No entity identifiers or search text.",
  tiles: [
    native("window sessions", "app opened", "Once per native workspace-window state after its analytics handshake; additional workspace windows count separately. Engine startups are excluded."),
    native("active identities", "app foregrounded", "Daily unique identities bringing the native window to the foreground.", { math: "dau" }),
    native("window session closes", "app closed", "Graceful quit or auxiliary workspace-window closure. Closing and reopening the primary window retains its session; crashes cannot report closure."),
    native("foreground transitions", "app foregrounded", "Deduplicated focus gains; repeated focus notifications do not count."),
    native("background transitions", "app backgrounded", "Focus losses and closure of the originating workspace window."),
    native("foreground seconds", "app active interval", "Monotonic foreground dwell, emitted at most once per minute plus focus loss/quit, each interval capped at 60 seconds. Consent/account epochs trim intervals.", { math: "sum", math_property: "active_seconds" }),
    native("screen visits", "screen viewed", "Route category transitions, including the initial screen; no render counts or raw routes.", { breakdown: "screen" }),
    native("settings visits", "settings viewed", "Section entries and revisits; unchanged section requests are excluded.", { breakdown: "section" }),
    native("palette opens", "palette opened", "Successful mounting of the command/search palette."),
    native("palette tabs", "palette tab changed", "Changes between finite palette tabs.", { breakdown: "tab" }),
    native("palette actions", "palette action invoked", "Activated result categories, including search-result selections; asynchronous commands may subsequently fail. No queries, result labels or paths.", { breakdown: "action" }),
    native("panel transitions", "panel changed", "Sidebar and surface-host explicit open/close state changes; automatic width fitting is excluded.", { breakdown: "panel" }),
    native("surface adoption", "surface selected", "Changes of the active right-pane tab, grouped by type; different tabs of the same type can each count.", { math: "dau", breakdown: "surface_kind" }),
    native("preference adoption", "preference changed", "Applied settings changes grouped by finite category; values, geometry saves and startup hydration are excluded.", { math: "dau", breakdown: "preference" }),
    native("chat openers", "chat opened", "Unique native navigation identities opening persisted chats.", { math: "dau" }),
    native("client operating systems", "app foregrounded", "Explicit desktop client OS, independently of the collecting engine host; older clients remain unknown.", { math: "dau", breakdown: "client_os" }),
    native("client architectures", "app foregrounded", "Explicit desktop client architecture; no inference from the remote engine.", { math: "dau", breakdown: "client_architecture" }),
    native("client package versions", "app foregrounded", "Numeric desktop workspace package version, independently of engine version; prerelease/build labels are excluded.", { math: "dau", breakdown: "client_version" }),
    native("onboarding steps", "onboarding step viewed", "Actual visible onboarding step entries; repeated requests for the current step do not count.", { breakdown: "step" }),
    native("onboarding outcomes", "onboarding finished", "Explicit completed or skipped wizard; automatic existing-user bypasses are excluded.", { breakdown: "outcome" }),
    tile("Zuse — Desktop activation funnel", "Same connected-engine identity starting a native workspace-window session, opening a chat, submitting and completing a turn within one day. Changing engines/accounts can break attribution; execution events are not labeled desktop.", {
      kind: "FunnelsQuery", dateRange: { date_from: "-30d" },
      series: [series("app opened", { surface: "desktop" }), series("chat opened", { surface: "desktop" }), series("message submitted"), series("turn completed")].map((node, order) => ({ ...node, order })),
      funnelsFilter: { funnelVizType: "steps", funnelOrderType: "ordered", funnelWindowInterval: 1, funnelWindowIntervalUnit: "day" },
    }),
  ],
});
