import * as BrowserCrypto from "@effect/platform-browser/BrowserCrypto";
import * as PgClient from "@effect/sql-pg/PgClient";
import {
	AccountConnectionId,
	AccountId,
	AppId,
	AppName,
	type AppSourceStorage,
	aesGcmCredentials,
	type BlobStorage,
	BlobStoreError,
	BuildId,
	createExecutor,
	Json as JsonValue,
	makeExecutorStorage,
	OwnerId,
	ProfileId,
	type Runtime,
	RuntimeBuildFailed,
	RuntimeProtocolFailed,
	runtimeAdapter,
	SourceError,
	SourceFiles,
	SourceSnapshot,
	ToolName,
} from "@executor-js/sdk/core";
import {
	defineApp,
	defineProvider,
	oauth2,
	object,
	secrets,
	string,
} from "apps";
import {
	DeclaredRequirements,
	type HostContext,
	HostedCatalog,
	HostedCatalogSummary,
	HostResponse,
	HostInspectError,
	HostCallError,
	ToolResultObservation,
} from "apps/contracts";
import { createAppHandler, hostContext } from "apps/host";
import { mcpRouter } from "apps/mcp";
import {
	Context,
	Effect,
	Layer,
	ManagedRuntime,
	Option,
	Redacted,
	Scheduler,
	Schema,
	Stream,
} from "effect";
import { FetchHttpClient, HttpClient } from "effect/unstable/http";
import type {
	EngineAuth,
	EngineConnection,
	EngineOptions,
	EnginePlugin,
	PluginEngine,
	PluginEngineErrorCode,
	StateCodec,
} from "./types.ts";

/** A failure the caller can explain; other failures stay opaque. */
export class PluginEngineError extends Error {
	constructor(
		readonly code: PluginEngineErrorCode,
		/** The SDK failure kind, for logs; never provider responses. */
		readonly detail?: { readonly tag: string; readonly reason?: string },
	) {
		super(code);
		this.name = "PluginEngineError";
	}
}
const CLIENT_REGISTRATION_REASONS = new Set([
	"client_registration_required",
	"client_not_approved",
	"client_metadata_rejected",
	"registration_rejected",
]);

const oauthSetupError = (error: unknown) => {
	if (typeof error !== "object" || error === null || !("_tag" in error))
		return error;
	const reason = "reason" in error ? error.reason : undefined;
	const detail = {
		tag: String(error._tag),
		...(typeof reason === "string" ? { reason } : {}),
	};
	if (
		error._tag === "OAuthClientUnavailable" ||
		(error._tag === "OAuthSetupFailed" &&
			typeof reason === "string" &&
			CLIENT_REGISTRATION_REASONS.has(reason))
	)
		return new PluginEngineError("client_registration_unsupported", detail);
	if (error._tag === "OAuthSetupFailed")
		return new PluginEngineError(
			reason === "service_unavailable" ? "unreachable" : "auth_unsupported",
			detail,
		);
	return error;
};

/** The network budget of the request an engine operation runs for. */
const RequestScope = Context.Reference<unknown>("zuse/executor/RequestScope", {
	defaultValue: () => undefined,
});
const STATE_CACHE_ENTRIES = 1024;
const STATE_CACHE_CHARS = 8 * 1024 * 1024;
/**
 * Immutable rows: content-addressed sources and per-deployment blobs. Workspace
 * heads are compare-and-set, so they are always read from the database.
 */
const cacheable = (id: string) =>
	id.startsWith("v2:blob:") || id.startsWith("v2:source:");
const encodeState = (value: unknown) =>
	JSON.stringify(value, (_key, item) =>
		item instanceof Uint8Array
			? { __executorBytes: Buffer.from(item).toString("base64") }
			: item,
	);
const decodeState = (text: string): unknown =>
	JSON.parse(text, (_key, value) =>
		value &&
		typeof value === "object" &&
		Object.keys(value).length === 1 &&
		typeof value.__executorBytes === "string"
			? new Uint8Array(Buffer.from(value.__executorBytes, "base64"))
			: value,
	);

/**
 * Sealed `zuse_executor_state` rows on the engine's SQL client, so a state
 * statement inside an SDK transaction uses that transaction's connection.
 * This process is the table's only writer. Immutable rows are read once and
 * kept (bounded); an unchanged immutable row is never rewritten. Nothing is
 * kept from inside a transaction, which may still roll back.
 */
export const sealedState = (sql: PgClient.PgClient, codec: StateCodec) => {
	const kept = new Map<string, string>();
	let keptChars = 0;
	/** Bumped by every write, so a read that raced one never keeps its row. */
	let writes = 0;
	const forget = (id: string) => {
		const text = kept.get(id);
		if (text === undefined) return;
		kept.delete(id);
		keptChars -= text.length;
	};
	const keep = (id: string, text: string) => {
		forget(id);
		if (!cacheable(id) || text.length > STATE_CACHE_CHARS / 8) return;
		kept.set(id, text);
		keptChars += text.length;
		for (const oldest of kept.keys()) {
			if (kept.size <= STATE_CACHE_ENTRIES && keptChars <= STATE_CACHE_CHARS)
				break;
			forget(oldest);
		}
	};
	const inTransaction = Effect.map(
		Effect.serviceOption(sql.transactionService),
		Option.isSome,
	);
	return {
		get: (id: string) =>
			Effect.gen(function* () {
				const transaction = yield* inTransaction;
				const hit = transaction ? undefined : kept.get(id);
				if (hit !== undefined) {
					kept.delete(id);
					kept.set(id, hit);
					return decodeState(hit);
				}
				const before = writes;
				const rows = yield* sql<{
					sealed: string;
				}>`SELECT sealed FROM zuse_executor_state WHERE id=${id}`;
				const row = rows[0];
				if (row === undefined) return undefined;
				const text = codec.open(id, row.sealed);
				if (!transaction && before === writes) keep(id, text);
				return decodeState(text);
			}),
		put: (id: string, value: unknown) =>
			Effect.gen(function* () {
				const text = encodeState(value);
				const transaction = yield* inTransaction;
				if (!transaction && kept.get(id) === text) return;
				writes++;
				forget(id);
				const sealed = codec.seal(id, text);
				yield* sql`INSERT INTO zuse_executor_state(id,sealed) VALUES(${id},${sealed}) ON CONFLICT(id) DO UPDATE SET sealed=excluded.sealed`;
				if (!transaction) {
					writes++;
					keep(id, text);
				}
			}),
		delete: (id: string) =>
			Effect.gen(function* () {
				writes++;
				forget(id);
				yield* sql`DELETE FROM zuse_executor_state WHERE id=${id}`;
				writes++;
			}),
	};
};

/**
 * SDK rows that every tool invocation reads: the app with its deployment,
 * the profile, the selected accounts and their OAuth grants.
 */
const CACHED_TABLES = new Set([
	"apps",
	"deployments",
	"profiles",
	"accounts",
	"oauthGrants",
]);
const ROW_CACHE_ENTRIES = 2048;
/** Matches the 60 s expiry of serve's plugin views. */
const ROW_TTL_MS = 60_000;
const TRACE = Symbol("trace");
class Unkeyable extends Error {}

/**
 * A structural key for ORM options, including their `where`/`join` builder
 * callbacks: each callback runs against a recording builder, so the key is
 * the exact sequence of builder calls and values. Anything else (classes,
 * byte arrays, coercion of a builder value) makes the read uncacheable.
 */
const optionKey = (value: unknown, depth = 0): unknown => {
	if (depth > 12) throw new Unkeyable();
	if (value === null || typeof value === "string" || typeof value === "boolean")
		return value;
	if (typeof value === "number") return Number.isFinite(value) ? value : String(value);
	if (value === undefined) return { u: 1 };
	if (value instanceof Date) return { d: value.getTime() };
	if (typeof value === "function") {
		const traced = (value as { [TRACE]?: unknown })[TRACE];
		if (traced !== undefined) return traced;
		const node = (trace: unknown): unknown =>
			new Proxy(() => undefined, {
				get: (_target, property) =>
					property === TRACE
						? trace
						: typeof property === "symbol"
							? undefined
							: node({ get: property, of: trace }),
				apply: (_target, _self, args: unknown[]) =>
					node({
						call: trace,
						args: args.map((arg) => optionKey(arg, depth + 1)),
					}),
			});
		return {
			fn: optionKey(
				(value as (builder: unknown) => unknown)(node({ arg: depth })),
				depth + 1,
			),
		};
	}
	if (Array.isArray(value)) return value.map((item) => optionKey(item, depth + 1));
	if (typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype)
		return Object.fromEntries(
			Object.entries(value).map(([key, item]) => [key, optionKey(item, depth + 1)]),
		);
	throw new Unkeyable();
};

/**
 * Keeps the SDK's per-invocation reads in memory between operations.
 *
 * Served only while every `executor_*` table carries the change trigger
 * (`0048_executor_wakeups.sql`) and serve forwards database change
 * notifications as an epoch with each request ({@link observe}); any other
 * process's write bumps it. This process's own ORM writes drop everything
 * before and after they run. Reads inside a transaction, and claimed or
 * missing OAuth grants (renewal coordination), always go to the database.
 */
export const rowCache = (sql: PgClient.PgClient) => {
	const rows = new Map<string, { readonly at: number; readonly value: unknown }>();
	/** Bumped by every invalidation; a read that raced one is not kept. */
	let generation = 0;
	/** Triggers verified on every executor table. */
	let installed = false;
	/** The latest epoch serve sent; undefined while serve cannot invalidate. */
	let epoch: number | undefined;
	const clear = () => {
		generation++;
		rows.clear();
	};
	const inTransaction = Effect.map(
		Effect.serviceOption(sql.transactionService),
		Option.isSome,
	);
	const keepable = (table: string, result: unknown) =>
		table !== "oauthGrants" ||
		(typeof result === "object" &&
			result !== null &&
			"status" in result &&
			typeof result.status === "string" &&
			result.status.startsWith("ready_"));
	const read =
		(method: string, table: string, work: Effect.Effect<unknown, unknown>) =>
		(options: unknown) =>
			Effect.gen(function* () {
				if (
					!installed ||
					epoch === undefined ||
					!CACHED_TABLES.has(table) ||
					(yield* inTransaction)
				)
					return yield* work;
				let key: string;
				try {
					key = JSON.stringify([method, table, optionKey(options)]);
				} catch {
					return yield* work;
				}
				const hit = rows.get(key);
				rows.delete(key);
				// Deployments never change; other rows are re-read after a while
				// in case a notification was lost unnoticed.
				if (
					hit !== undefined &&
					(table === "deployments" || Date.now() - hit.at < ROW_TTL_MS)
				) {
					rows.set(key, hit);
					return structuredClone(hit.value);
				}
				const before = generation;
				const result = yield* work;
				if (before === generation && keepable(table, result)) {
					try {
						rows.set(key, { at: Date.now(), value: structuredClone(result) });
					} catch {
						return result;
					}
					for (const oldest of rows.keys()) {
						if (rows.size <= ROW_CACHE_ENTRIES) break;
						rows.delete(oldest);
					}
				}
				return result;
			});
	const writing = <A, E, R>(effect: Effect.Effect<A, E, R>) =>
		Effect.suspend(() => {
			clear();
			return effect;
		}).pipe(Effect.ensuring(Effect.sync(clear)));
	return {
		/** Checks once that every executor table notifies on change. */
		verify: Effect.gen(function* () {
			const rows = yield* sql<{
				tables: string;
				missing: string;
			}>`SELECT count(*) AS tables, count(*) FILTER (WHERE NOT EXISTS (SELECT 1 FROM pg_trigger g WHERE g.tgrelid=c.oid AND g.tgname='zuse_cloud_executor_wake' AND g.tgenabled<>'D')) AS missing FROM pg_class c WHERE c.relnamespace=current_schema()::regnamespace AND c.relkind='r' AND left(c.relname,9)='executor_'`;
			const found = rows[0];
			installed =
				found !== undefined &&
				Number(found.tables) > 0 &&
				Number(found.missing) === 0;
			if (!installed)
				console.error(
					JSON.stringify({ event: "executor_row_cache_disabled", reason: "triggers_missing" }),
				);
		}),
		/**
		 * Serve's change epoch for this request: `null` while serve is not
		 * listening (read everything); a newer epoch drops every kept row.
		 */
		observe(next: number | null) {
			if (next === null) {
				if (epoch !== undefined) clear();
				epoch = undefined;
			} else if (epoch === undefined || next > epoch) {
				clear();
				epoch = next;
			}
		},
		wrap<O extends object>(source: O): O {
			const orm = source as Record<string, unknown>;
			const wrapped: Record<string, unknown> = { ...orm };
			for (const method of ["findFirst", "findMany", "count"])
				wrapped[method] = (table: string, options?: unknown) =>
					read(
						method,
						table,
						(orm[method] as (t: string, o?: unknown) => Effect.Effect<unknown, unknown>)(
							table,
							options,
						),
					)(options);
			for (const method of [
				"create",
				"createMany",
				"updateMany",
				"deleteMany",
				"upsert",
			])
				wrapped[method] = (...args: unknown[]) =>
					writing(
						(orm[method] as (...a: unknown[]) => Effect.Effect<unknown, unknown>)(
							...args,
						),
					);
			wrapped.transaction = (effect: Effect.Effect<unknown, unknown, unknown>) =>
				writing(
					(orm.transaction as (e: typeof effect) => Effect.Effect<unknown, unknown>)(
						effect,
					),
				);
			return wrapped as unknown as O;
		},
	};
};

/**
 * Only these bundled declarations can execute: a remote MCP endpoint with one
 * auth method. Endpoints come from the host's lookup; no user source, eval, or
 * dynamic deployment.
 */
export async function createPluginEngine(
	options: EngineOptions,
): Promise<PluginEngine> {
	// One pool for the SDK tables and the sealed state rows. Statements of a
	// transaction reuse its connection, so one connection cannot deadlock.
	const pool = PgClient.layer({
		url: Redacted.make(options.databaseUrl),
		maxConnections: Math.max(1, Math.floor(options.maxConnections ?? 1)),
		idleTimeout: "60 seconds",
		connectionTTL: "30 minutes",
		connectTimeout: "5 seconds",
		applicationName: "zuse-cloud-executor",
	});
	const services = Layer.mergeAll(
		pool,
		BrowserCrypto.layer,
		FetchHttpClient.layer.pipe(
			Layer.provide(Layer.succeed(FetchHttpClient.Fetch, options.fetch)),
		),
	);
	const managed = ManagedRuntime.make(services);
	/**
	 * The engine and its pool outlive requests, so a fiber can be resumed by a
	 * database connection another request opened, in that request's async
	 * context. Each operation therefore pins its caller's network budget: its
	 * fibers yield and continue inside the budget whenever they find
	 * themselves outside it, the SDK's HTTP client gets a fetch bound to it,
	 * and app handlers run re-entered in it.
	 */
	const run = <A, E, R extends Layer.Success<typeof services>>(
		effect: Effect.Effect<A, E, R>,
	) => {
		const scope = options.requestScope.capture();
		const enter = <T>(work: () => T) =>
			options.requestScope.enter(scope, work);
		const base = new Scheduler.MixedScheduler();
		const pinned: Scheduler.Scheduler = {
			executionMode: base.executionMode,
			shouldYield: (fiber) =>
				base.shouldYield(fiber) ||
				(scope !== undefined && options.requestScope.capture() !== scope),
			makeDispatcher: () => {
				const dispatcher = base.makeDispatcher();
				return {
					scheduleTask: (task, priority) =>
						dispatcher.scheduleTask(() => enter(task), priority),
					flush: () => dispatcher.flush(),
				};
			},
		};
		const fetch = ((input: RequestInfo | URL, init?: RequestInit) =>
			enter(() => options.fetch(input, init))) as typeof globalThis.fetch;
		return managed.runPromise(
			effect.pipe(
				Effect.provideService(RequestScope, scope),
				Effect.provideService(FetchHttpClient.Fetch, fetch),
				Effect.provideService(Scheduler.Scheduler, pinned),
			),
		);
	};
	const io = <A>(work: () => Promise<A>) =>
		Effect.tryPromise({ try: work, catch: () => new RuntimeProtocolFailed() });
	type Entry = {
		readonly source: string;
		readonly build: BuildId;
		readonly handler: ReturnType<typeof createAppHandler>;
	};
	const entries = new Map<string, Promise<Entry>>();
	const builds = new Map<BuildId, Entry>();
	const definition = (pluginId: string) => {
		const plugin = options.plugin(pluginId);
		if (!plugin) throw new Error("Unknown plugin");
		return plugin;
	};
	const headerOf = (plugin: EnginePlugin) => plugin.header ?? "Authorization";

	const sourceOf = (plugin: EnginePlugin, auth: EngineAuth) =>
		`export default ${JSON.stringify({ protocol: 1, id: plugin.id, name: plugin.name, endpoint: plugin.endpoint, auth, scopes: plugin.scopes ?? [], ...(plugin.oauthDiscovery ? { oauthDiscovery: plugin.oauthDiscovery } : {}), ...(auth === "header" ? { header: headerOf(plugin) } : {}) })};\n`;

	// The engine outlives requests: entries are keyed by the generated source,
	// which embeds the tenant-qualified id and every endpoint field, so an
	// edited definition is a new entry. Both maps stay bounded.
	const MAX_ENTRIES = 512;
	const bounded = <K, V>(map: Map<K, V>) => {
		for (const oldest of map.keys()) {
			if (map.size <= MAX_ENTRIES) break;
			map.delete(oldest);
		}
	};
	const register = (pluginId: string, auth: EngineAuth): Promise<Entry> => {
		const source = sourceOf(definition(pluginId), auth);
		const existing = entries.get(source);
		if (existing)
			return existing.then((entry) => {
				if (builds.get(entry.build) !== entry) {
					builds.delete(entry.build);
					builds.set(entry.build, entry);
					bounded(builds);
				}
				return entry;
			});
		const created = (async () => {
			const plugin = definition(pluginId);
			const digest = await crypto.subtle.digest(
				"SHA-256",
				new TextEncoder().encode(source),
			);
			const build = BuildId.make(
				`bld_${Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("")}`,
			);
			const header = headerOf(plugin);
			const app =
				auth === "none"
					? defineApp({ accounts: {} }, async ({ signal }) => ({
							tools: await mcpRouter({ url: plugin.endpoint, signal }),
						}))
					: auth === "header"
						? defineApp(
								{
									accounts: {
										service: defineProvider({
											name: plugin.name,
											auth: {
												key: secrets({
													label: header,
													fields: object({ value: string({ minLength: 1 }) }),
												}),
											},
										}),
									},
								},
								async ({ accounts, signal }) => ({
									tools: await mcpRouter({
										url: plugin.endpoint,
										signal,
										headers: { [header]: accounts.service.fields.value },
										accountId: accounts.service.id,
									}),
								}),
							)
						: defineApp(
								{
									accounts: {
										service: defineProvider({
											name: plugin.name,
											auth: {
												oauth: oauth2({
													discover: plugin.oauthDiscovery ?? plugin.endpoint,
													scopes: [...(plugin.scopes ?? [])],
												}),
											},
										}),
									},
								},
								async ({ accounts, signal }) => ({
									tools: await mcpRouter({
										url: plugin.endpoint,
										signal,
										accountId: accounts.service.id,
										headers: {
											Authorization: `Bearer ${accounts.service.fields.access_token}`,
										},
									}),
								}),
							);
			const entry = { source, build, handler: createAppHandler(app) };
			builds.delete(build);
			builds.set(build, entry);
			bounded(builds);
			return entry;
		})();
		entries.set(source, created);
		bounded(entries);
		created.catch(() => entries.delete(source));
		return created;
	};

	const entryForSource = async (source: string | undefined) => {
		if (source === undefined) return undefined;
		for (const entry of builds.values())
			if (entry.source === source) return entry;
		const declared = /^export default (\{.*\});\n$/s.exec(source)?.[1];
		if (!declared) return undefined;
		const parsed: unknown = JSON.parse(declared);
		if (typeof parsed !== "object" || parsed === null) return undefined;
		const { id, auth } = parsed as { id?: unknown; auth?: unknown };
		if (
			typeof id !== "string" ||
			(auth !== "none" && auth !== "oauth" && auth !== "header") ||
			!options.plugin(id)
		)
			return undefined;
		const entry = await register(id, auth);
		return entry.source === source ? entry : undefined;
	};
	const dispatch = <E>(
		build: string,
		command: unknown,
		error: Schema.Decoder<E>,
		context: HostContext = hostContext({}),
	) =>
		Effect.flatMap(Effect.service(RequestScope), (scope) =>
			// Re-enter the caller's budget: the app's MCP client uses global fetch.
			io(() =>
				options.requestScope.enter(scope, async () => {
					const item = builds.get(BuildId.make(build));
					if (!item) throw new Error("Unknown plugin build");
					const response = await item.handler(
						new Request("https://zuse.invalid/", {
							method: "POST",
							body: JSON.stringify(command),
							headers: { "content-type": "application/json" },
							signal: AbortSignal.timeout(30_000),
						}),
						context,
					);
					return Schema.decodeUnknownSync(HostResponse)(await response.json());
				}),
			),
		).pipe(
			Effect.flatMap((result) => {
				if (!result.ok)
					return Schema.decodeUnknownEffect(error)(result.error).pipe(
						Effect.mapError(() => new RuntimeProtocolFailed()),
						Effect.flatMap(Effect.fail),
					);
				return Effect.gen(function* () {
					if (result.toolError === true)
						(yield* ToolResultObservation).failed();
					return result.value;
				});
			}),
		);
	const decode = <A>(schema: Schema.Decoder<A>) =>
		Effect.flatMap((value: unknown) =>
			Schema.decodeUnknownEffect(schema)(value).pipe(
				Effect.mapError(() => new RuntimeProtocolFailed()),
			),
		);
	const unsupported = () => Effect.fail(new RuntimeProtocolFailed());
	const runtime: Runtime = {
		build: ({ files }) =>
			Effect.gen(function* () {
				const source = files.find((file) => file.path === "index.ts")?.content;
				const entry = yield* Effect.tryPromise({
					try: () => entryForSource(source),
					catch: () => new RuntimeBuildFailed({ stage: "compile" }),
				});
				if (!entry) return yield* new RuntimeBuildFailed({ stage: "compile" });
				const requirements = yield* dispatch(
					entry.build,
					{
						operation: "requirements",
					},
					HostInspectError,
				).pipe(
					decode(DeclaredRequirements),
					Effect.mapError(() => new RuntimeBuildFailed({ stage: "compile" })),
				);
				return { build: entry.build, requirements };
			}),
		skills: () => Effect.succeed({ skills: [] }),
		inspect: ({ build, tools, ...context }) =>
			dispatch(
				build,
				{ operation: "inspect", ...(tools ? { tools } : {}) },
				HostInspectError,
				context,
			).pipe(decode(HostedCatalog)),
		index: ({ build, ...context }) =>
			dispatch(
				build,
				{ operation: "inspect", detail: "summary" },
				HostInspectError,
				context,
			).pipe(decode(HostedCatalogSummary)),
		call: ({ build, tool, input, kind, ...context }) =>
			dispatch(
				build,
				{
					operation: "call",
					tool,
					input,
					...(kind === undefined ? {} : { kind }),
				},
				HostCallError,
				context,
			).pipe(decode(JsonValue)),
		query: unsupported,
		mutate: unsupported,
		webhook: unsupported,
		workflow: unsupported,
		checkAccount: unsupported,
		changes: () => Stream.empty,
	};
	try {
		const sql = await run(PgClient.PgClient);
		const state = sealedState(sql, options.state);
		const blobs: BlobStorage = {
			get: (key) =>
				state.get(`v2:blob:${key}`).pipe(
					Effect.map((value) => Option.fromNullishOr(value as Uint8Array)),
					Effect.mapError(() => new BlobStoreError({ operation: "get" })),
				),
			put: (key, body) =>
				state
					.put(`v2:blob:${key}`, body)
					.pipe(Effect.mapError(() => new BlobStoreError({ operation: "put" }))),
			remove: (key) =>
				state
					.delete(`v2:blob:${key}`)
					.pipe(
						Effect.mapError(() => new BlobStoreError({ operation: "remove" })),
					),
		};
		const sourceError = () => new SourceError({ reason: "storage" });
		const retain: AppSourceStorage["retain"] = (code, files) =>
			Effect.gen(function* () {
				const digest = yield* Effect.tryPromise({
					try: () =>
						crypto.subtle.digest(
							"SHA-1",
							new TextEncoder().encode(JSON.stringify(files)),
						),
					catch: sourceError,
				});
				const commit = Array.from(new Uint8Array(digest), (b) =>
					b.toString(16).padStart(2, "0"),
				).join("");
				yield* state
					.put(`v2:source:${code}:${commit}`, files)
					.pipe(Effect.mapError(sourceError));
				return { code, commit };
			});
		const sources: AppSourceStorage = {
			retain,
			read: ({ code, commit }) =>
				state.get(`v2:source:${code}:${commit}`).pipe(
					Effect.flatMap((value) =>
						Effect.try({
							try: () => Schema.decodeUnknownSync(SourceFiles)(value),
							catch: sourceError,
						}),
					),
					Effect.mapError(sourceError),
				),
			workspace: (code) =>
				state.get(`v2:head:${code}`).pipe(
					Effect.flatMap((value) =>
						Effect.try({
							try: () =>
								value === undefined
									? null
									: Schema.decodeUnknownSync(SourceSnapshot)(value),
							catch: sourceError,
						}),
					),
					Effect.mapError(sourceError),
				),
			commit: (input) =>
				Effect.gen(function* () {
					const old = yield* sources.workspace(input.code);
					if ((old?.revision.commit ?? null) !== input.expected)
						return yield* new SourceError({ reason: "conflict" });
					const revision = yield* retain(input.code, input.files);
					const next = { revision, files: input.files };
					yield* state
						.put(`v2:head:${input.code}`, next)
						.pipe(Effect.mapError(sourceError));
					return next;
				}),
		};
		const storage = await run(makeExecutorStorage({ provider: "postgresql" }));
		const cache = rowCache(sql);
		const db: typeof storage = {
			...storage,
			orm: (version) => cache.wrap(storage.orm(version)),
		};
		if (options.migrate) await run(db.migrate);
		else {
			await run(db.checkMigration);
			await run(cache.verify);
		}
		const bytes = Uint8Array.from(
			atob(options.encryptionKey.replaceAll("-", "+").replaceAll("_", "/")),
			(c) => c.charCodeAt(0),
		);
		if (bytes.length !== 32) throw new Error("Invalid plugin encryption key");
		const material = await crypto.subtle.importKey(
			"raw",
			bytes,
			"HKDF",
			false,
			["deriveBits"],
		);
		const scoped = await crypto.subtle.deriveBits(
			{
				name: "HKDF",
				hash: "SHA-256",
				salt: new TextEncoder().encode(options.credentialScope),
				info: new TextEncoder().encode("zuse-plugins-v2"),
			},
			material,
			256,
		);
		const hex = Array.from(new Uint8Array(scoped), (b) =>
			b.toString(16).padStart(2, "0"),
		).join("");
		const credentials = await run(
			aesGcmCredentials(Redacted.make(hex), crypto),
		);
		const client = await run(HttpClient.HttpClient);
		const e = await run(
			createExecutor({
				storage: db,
				sources,
				blobs,
				credentials,
				runtime: runtimeAdapter(runtime),
				// Evaluate every listing, as when each request opened its own
				// engine: a long-lived engine must not serve remembered catalogs.
				toolListings: {
					freshMillis: 0,
					maxStaleMillis: 0,
					loadMillis: 0,
					slowFailureMillis: 1_000,
				},
				oauth: {
					httpClient: client,
					clientName: "Zuse",
					urlPolicy: {
						allowLoopbackHttp: options.allowLoopbackHttp ?? false,
						allowedHttpOrigins: [],
					},
				},
			}),
		);
		await run(e.apps.list({ owner: OwnerId.make("__readiness__") }));
		const ownerOf = (owner: string) => OwnerId.make(owner);

		/**
		 * (owner, app, profile) triples whose ownership was verified. An app's
		 * owner and a profile's app never change, and every tool read loads the
		 * app and profile again (removed or disabled ones fail there), so tool
		 * reads verify ownership once per process. Bounded; setup always checks.
		 */
		const owned = new Map<string, string>();
		const target = async (
			owner: string,
			ref: EngineConnection,
			read = false,
		) => {
			await register(ref.plugin, ref.auth);
			const app = AppId.make(ref.app),
				profile = ProfileId.make(ref.profile);
			const key = JSON.stringify([owner, ref.app, ref.profile]);
			if (read && owned.has(key)) return { app, profile };
			await run(e.apps.get({ app, owner: ownerOf(owner) }));
			await run(e.apps.profiles.get({ app, profile, owner: ownerOf(owner) }));
			if (read) {
				owned.set(key, ref.app);
				bounded(owned);
			}
			return { app, profile };
		};
		return {
			async prepare(owner, subject, id, pluginId, auth) {
				const entry = await register(pluginId, auth);
				const name = AppName.make(id),
					own = ownerOf(owner);
				const existing = await run(e.apps.list({ owner: own, name }));
				const app =
					existing[0] ??
					(
						await run(
							e.apps.deploy({
								owner: own,
								name,
								files: [{ path: "index.ts", content: entry.source }],
							}),
						)
					).app;
				const profile = await run(
					e.apps.profiles.create({
						app: app.id,
						owner: own,
						subject,
						accounts: {},
						idempotencyKey: id,
					}),
				);
				const ref = {
					app: app.id,
					profile: profile.id,
					plugin: pluginId,
					auth,
				};
				if (auth === "none") return ref;
				const connection = await run(
					e.accountConnections.create({
						owner: own,
						target: {
							app: app.id,
							profile: profile.id,
							requirement: "service",
						},
					}),
				);
				return { ...ref, connection: connection.id };
			},
			async start(owner, ref, label, redirectUri) {
				await target(owner, ref);
				if (!ref.connection) throw new Error("Missing connection");
				const result = await run(
					e.accountConnections.startOAuth({
						owner: ownerOf(owner),
						connection: AccountConnectionId.make(ref.connection),
						method: "oauth",
						label,
						redirectUri,
					}),
				).catch((error: unknown) => {
					throw oauthSetupError(error);
				});
				if (result.status !== "redirect")
					throw new Error("Expected authorization redirect");
				return result.authorizationUrl;
			},
			async submit(owner, ref, label, value) {
				await target(owner, ref);
				if (!ref.connection) throw new Error("Missing connection");
				const account = await run(
					e.accountConnections.submit({
						owner: ownerOf(owner),
						connection: AccountConnectionId.make(ref.connection),
						method: "key",
						label,
						fields: Redacted.make({ value }),
					}),
				);
				return { ...ref, account: account.id };
			},
			async complete(owner, ref, callbackUrl) {
				await target(owner, ref);
				if (!ref.connection) throw new Error("Missing connection");
				const result = await run(
					e.accountConnections.completeOAuth({
						owner: ownerOf(owner),
						connection: AccountConnectionId.make(ref.connection),
						callbackUrl: Redacted.make(callbackUrl),
					}),
				);
				return { ...ref, account: result.id };
			},
			async remove(owner, id, ref) {
				const own = ownerOf(owner);
				const app =
					ref?.app ??
					(await run(e.apps.list({ owner: own, name: AppName.make(id) })))[0]
						?.id;
				let account = ref?.account;
				if (ref?.connection) {
					const cancelled = await run(
						e.accountConnections.cancel({
							owner: own,
							connection: AccountConnectionId.make(ref.connection),
						}),
					);
					if (cancelled.state.status === "completed")
						account = cancelled.state.account.id;
				}
				if (app) {
					for (const [key, kept] of owned) if (kept === app) owned.delete(key);
					await run(e.apps.remove({ app: AppId.make(app), owner: own }));
				}
				if (account)
					await run(
						e.accounts.remove({ account: AccountId.make(account), owner: own }),
					);
			},
			async list(owner, ref) {
				const t = await target(owner, ref, true);
				const result = await run(e.tools.index(t));
				return result.items.map((tool) => ({
					name: tool.name,
					description: tool.description,
				}));
			},
			async schema(owner, ref, tool) {
				return run(
					e.tools.get({
						...(await target(owner, ref, true)),
						tool: ToolName.make(tool),
					}),
				);
			},
			async call(owner, ref, tool, input) {
				const result = await run(
					e.tools.call({
						...(await target(owner, ref, true)),
						tool: ToolName.make(tool),
						input: Schema.decodeUnknownSync(JsonValue)(input),
					}),
				);
				if (result.status !== "completed")
					throw new Error("Plugin requires additional approval");
				return result.value;
			},
			observe: (epoch) => cache.observe(epoch),
			ready: () =>
				run(sql`SELECT 1 FROM zuse_executor_state LIMIT 1`).then(() => undefined),
			close: () => managed.dispose(),
		};
	} catch (error) {
		await managed.dispose();
		throw error;
	}
}
