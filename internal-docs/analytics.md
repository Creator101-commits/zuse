# PostHog product analytics

Explicit product events cover chat lifecycle, client navigation, models, messages,
settings, execution, reliability, engagement, and feature adoption. This follows
`pingdotgg/t3code`'s distinction between client actions and provider execution,
adapted to the Rust engine, native desktop, Solid browser, and Expo clients.

## Questions the instrumentation answers

| Question | Event or dimension | Interpretation |
| --- | --- | --- |
| How many chats were created? | `chat created` | New conversation rows committed locally, including new conversation tabs; retries creating the same id are excluded |
| How many chats were opened? | `chat opened` | Navigation visits, including revisits, separated by desktop/web/iOS/Android; reconnecting a transcript does not count as opening |
| How many people create or open chats? | Unique event `distinct_id` | Pseudonymous account/install identities, not exact people |
| How many messages are submitted? | `message submitted` | Persisted run/steer commands; duplicate message ids present in the command ledger are excluded |
| Which providers and models are selected? | `provider`, `model` | Names from trusted vendor catalogs; custom/local identifiers are `custom`, provider-selected defaults are `default` |
| Which model actually runs? | `actual_model` on turn outcomes | Last model reported by the main provider; absent when not reported; `mixed_models` flags more than one reported model within the turn |
| Which reasoning/access settings are used? | `reasoning_effort`, `permission_mode`, `plan_mode` | Typed settings at submission or execution; default effort is labeled `default` |
| How often do users change models? | `model changed` | Successful provider/model configuration changes, with `previous_provider` and `previous_model`; unchanged writes are excluded |
| What other settings change? | `chat settings changed` | Successful settings changes while provider/model stay the same |
| How many chats are archived, restored, or deleted? | `chat archived`, `chat restored`, `chat deleted` | Successful lifecycle transitions/removals, not merely RPC requests |
| How often do turns succeed or fail? | `turn started`, `turn completed`, `turn failed`, `turn interrupted` | Main-agent execution; provider continuations may begin without a new message submission |
| How long do turns take? | `duration_ms`, `first_token_ms` | Main-agent elapsed time and first nonempty text latency; includes tool/approval waits; tool-only turns omit first text latency |
| How much tool activity occurs? | `tool_count`, `tool_failure_count`, category counts | Unique tool/result ids per turn, deduplicated in memory; no tool input/output is sent |
| Which tool categories are used? | `shell_tool_count`, `files_tool_count`, `browser_tool_count`, `mcp_tool_count`, `subagent_tool_count`, `other_tool_count` | Typed categories; raw tool/server names are excluded |
| How much subagent activity occurs? | `subagent`, `subagent_count`, `subagent_completed_count` | Side-chat flag plus observed spawn references/completions during the main turn; background completions after parent turn end are not retroactively counted |
| How much usage is observed? | `input_tokens`, `output_tokens`, `usage_status`, `usage_scope` | Provider snapshots, or summed native main-agent requests; missing stays absent, not zero |
| Are cache/reasoning measurements available? | `cache_read_tokens`, `cache_creation_tokens`, `reasoning_tokens` | Optional native request-accounting measurements; absent on drivers without normalized evidence |
| How many request attempts fail? | `request_attempt_count`, `request_failure_count` | Native request-accounting aggregates, deduplicated by request/retry identity in memory; no identifiers/raw errors leave the process |
| How often are approvals and interrupts requested? | `permission answered`, `turn interrupt requested`, `input_request_count` | Durable control commands and observed main-agent input requests |
| How many active installations exist? | Distinct `install_id` on `message submitted` | Stable random installation identity, independent of account sign-in and anonymous-id rotation |
| Which systems and releases are active? | `os`, `architecture`, `app_version` | Engine execution host; these are not guessed from the connected client |
| What features are adopted? | `feature used` with `feature`, `outcome`, `duration_ms` | Closed list of feature RPC attempts; distinct from durable chat/message counts |
| How engaged are browser users? | `screen viewed`, `app active interval` | Closed screen categories and focused/visible active seconds; idle after 60 seconds |

Conversation identifiers, titles, and content are not collected, so an open count
is a count of visits, not the number of distinct historical conversations opened.
Created-chat counts and unique opener identities remain available. Counts begin
with deployment; historical activity cannot be reconstructed.

## Model catalog boundary

The engine obtains safe Codex/Claude model names directly from the bundled vendor
catalog functions in `crates/providers`. It never trusts Claude's configured
model overrides, user-added model preferences, custom ACP instance identifiers,
or arbitrary OpenCode/Pi model catalogs. Cursor and native vendor inventory
models are registered when their model list is successfully loaded, before
custom picker rows are merged. Unregistered names are grouped under `custom`.
A new vendor model may therefore appear as `custom` until its trusted catalog is
available. The browser/mobile shared property filter remains conservative; model
reporting uses engine lifecycle/submission/execution events.

## Accuracy and attribution

`chat created` is recorded after successful row creation, including the separate
conversation-tab creation path. Repeated creates of an existing id record
nothing. Model/configuration comparisons occur under the registry mutation lock;
failed compare-and-set attempts do not emit changes. Archive/restore events fire
only when the saved archived value changes. Command analytics follow persistence
and the command/message-id duplicate checks. These events are separate from
`feature used`, which counts observed RPC attempts and can appear on both engines
when a request is forwarded.

Client chat-open events occur at navigation, not transcript subscription. Native
and browser clients report to the connected engine; its consent controls delivery.
Browser consent also gates its navigation reports. Expo records navigation
through its existing consent-gated SDK. Client surfaces are explicit; execution
events retain `surface=engine` rather than inventing a requesting client.

A turn includes selected model/settings and, when reported, the actual model.
Unknown values remain default/custom/absent rather than being inferred. Codex,
Claude, and Cursor usage events are provider snapshots; native usage sums
main-agent requests. Nested-agent usage does not get added to parent totals.
`usage_status=observed` does **not** assert billing completeness. Cache/reasoning
and request-attempt counters are optional native observations. Compare matched
model/settings/outcome cohorts and check usage coverage, subagent activity and
`mixed_models`. Cost, price estimates, and a complete nested-agent token total
are not inferred. The current command protocol does not reliably distinguish all
human, agent, and scheduled senders, so message-based active users are described
as active submitters rather than verified human users.

The registry and command-ledger checks exclude ordinary retries in one local
runtime. Analytics are best effort, not a globally serialized audit log: distinct
writers creating the same row concurrently, command pruning, multiple devices,
opt-outs, blockers, and identity rotation can affect counts. Mirrored/imported
historical rows are not backfilled as newly created chats.

## Configuration and consent

Use the public project ingest token from PostHog project settings. See
`.env.analytics.example`. Share one project across engine/browser/mobile when
cross-platform reporting is wanted.

- Rust reads `ZUSE_POSTHOG_KEY` and `ZUSE_POSTHOG_HOST` at runtime, falling back to
  values embedded when Cargo compiled the analytics crate. Export these during
  release builds; Cargo does not automatically read dotenv files.
- Vite reads `VITE_POSTHOG_*` at build time or from `apps/web-solid/.env.local`.
- Expo uses `EXPO_PUBLIC_POSTHOG_*` in its build environment.
- US ingest defaults to `https://us.i.posthog.com`; EU projects need
  `https://eu.i.posthog.com`.
- Missing keys disable delivery. Debug Rust/Vite builds also require their
  `*_POSTHOG_ENABLE_DEV=1`; use a separate test project. The engine override
  `ZUSE_ANALYTICS_ENABLED=0` disables collection regardless of saved preference.

Engine consent is read from `<data-dir>/analytics.json` before the first capture.
It defaults on for new installations. Corrupt preferences or unavailable storage
fail closed. Native **Settings → General → Privacy → Share usage analytics** and
browser **Share engine usage analytics** update this preference on the connected
engine. Browser **Share browser usage analytics** is separately persisted in
localStorage. Do Not Track and Global Privacy Control prevent browser SDK
initialization. Expo retains its existing explicit opt-in flow.

Signed-out installs use random anonymous UUIDs. Signed-in events use SHA-256 of
`zuse:analytics:account:v1:<account-id>`, prefixed by `account_`. Sign-out rotates
the anonymous identity while `install_id` stays stable. Identity/consent changes
invalidate queued engine events. PostHog person-profile processing is disabled.

## Dashboards

`scripts/analytics-dashboards.mjs` defines four dashboards:

1. **Zuse — Chats and activation**: creation, opening, messages, lifecycle and
   creation-to-submission-to-completion funnel.
2. **Zuse — Models and settings**: model/provider adoption, actual models,
   reasoning/access/plan modes, and switching.
3. **Zuse — Execution and reliability**: outcomes, duration/first-text percentiles,
   tool categories/failures, observed tokens, usage coverage and subagents.
4. **Zuse — Engagement and environment**: daily/weekly submitters, installations,
   host OS/architecture/version, feature adoption, and browser engagement.

Preview definitions without credentials or network requests:

```sh
node scripts/provision-analytics.mjs > /tmp/zuse-analytics-dashboards.json
```

To upsert the dashboards and saved insights in a configured project, set
`POSTHOG_PROJECT_ID`, `POSTHOG_PERSONAL_API_KEY` and optionally
`POSTHOG_CONTROL_HOST` (`https://us.posthog.com` or `https://eu.posthog.com`), then:

```sh
node scripts/provision-analytics.mjs --apply
```

Operations keys need dashboard/insight read and write scopes and must never be
bundled with an app. Re-running updates named objects and retains unrelated
insight dashboard memberships. Provisioning uses the official
[dashboard API](https://posthog.com/docs/api/dashboards) and
[insight API](https://posthog.com/docs/api/insights). Definitions are not live
until applied to a project. Live query validation requires that project's
credentials and deployed events.

All reports filter on `analytics_schema_version=2` and explicit surfaces.
New detailed events/properties are additive; pre-expansion history does not gain
missing model or chat-open dimensions. Daily unique counts must not be summed
into a monthly unique-user total; users can appear on several clients/hosts.

## Privacy, delivery, and checks

Payload filters run before engine delivery and in the browser SDK's `before_send`
hook. Prompts, responses, reasoning text, code, tool arguments/results, repository
names, paths, URLs, chat/message/request/account identifiers, user-defined model
names, credentials, raw errors, stacks and SDK-added referrers/profile updates
are excluded. Only random install identities and namespaced account hashes are
used for analytics identity. Input length is bucketed locally. No session replay,
DOM autocapture, exception capture, or analytics feature-flag dependency is enabled.
Standard PostHog network/geographic processing still applies.

The engine queues at most 512 events and sends up to 50 per batch every 10 seconds.
Failed requests get at most three attempts with cancellable backoff and unchanged
UUIDs. Permanent client errors are dropped. Overflow and failed batches are
best-effort drops; there is no disk outbox or historical backfill. Shutdown waits
at most three seconds after application work stops. Already transmitted or
in-flight events cannot be recalled by opt-out.

Validation commands:

```sh
cargo test -p zuse-analytics
cargo check -p zuse-engine
node --test scripts/analytics-dashboards.test.mjs
bun test --cwd packages/analytics
cd apps/web-solid
bun run typecheck
bun run lint
bun run e2e:analytics
bun node_modules/vite/bin/vite.js build
```

The Rust tests check actual loopback batch payloads, consent, model-name privacy,
identity rotation, tool deduplication and usage aggregation. The browser smoke
uses the real slim SDK with intercepted beacons and checks navigation counts,
reconnects, identity, opt-out/re-enable, persisted consent, and GPC. Dashboard
provisioning tests use a mock API and verify repeatability and credential scoping.
Native UI checks require the repository's graphics/audio development libraries;
Expo checks require its full mobile workspace dependencies.
