/** Private process boundary. The public Rust API authenticates every caller. */
import { createServer } from "node:http";
import { AsyncLocalStorage } from "node:async_hooks";
import {
	createHash,
	randomBytes,
	createCipheriv,
	createDecipheriv,
	timingSafeEqual,
} from "node:crypto";
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import {
	CallToolRequestSchema,
	ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { createPluginEngine } from "../vendor/runtime.mjs";
import { PLUGIN_MCP_TOOLS as tools } from "./tools.ts";
import type {
	EngineConnection,
	EnginePlugin,
	PluginEngine,
	StateCodec,
} from "./types.ts";

process.on("uncaughtException", () => {
	console.error("executor_process_failed");
	process.exit(1);
});
process.on("unhandledRejection", () => {
	console.error("executor_process_failed");
	process.exit(1);
});
const databaseUrl = process.env.ZUSE_EXECUTOR_DATABASE_URL!;
const encryptionKey = process.env.ZUSE_EXECUTOR_ENCRYPTION_KEY!;
const token = process.env.ZUSE_EXECUTOR_TOKEN!;
const proxy = process.env.ZUSE_EXECUTOR_EGRESS!;
if (!databaseUrl || !encryptionKey || !token || !proxy)
	throw new Error("Executor configuration missing");
// Every SDK and state statement shares this many connections (default 1).
const rawMaxConnections = process.env.ZUSE_EXECUTOR_DB_MAX_CONNECTIONS ?? "1";
const maxConnections = Number(rawMaxConnections);
if (
	!/^[0-9]+$/.test(rawMaxConnections) ||
	maxConnections < 1 ||
	maxConnections > 16
)
	throw new Error("Invalid executor connection limit");
const key = Buffer.from(encryptionKey, "base64url");
const nativeFetch = globalThis.fetch;
type Budget = {
	bytes: number;
	requests: number;
	signal: AbortSignal;
	/** False once the request settled; its budget never pays for later work. */
	open: boolean;
};
const budget = new AsyncLocalStorage<Budget>();
const MAX_BYTES = 8 * 1024 * 1024;

// Bound all HTTP (including SDK transports using global fetch) through the Rust
// resolver. Credentials never reach an unchecked URL or environment proxy.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
	const state = budget.getStore();
	if (!state || !state.open || ++state.requests > 1100)
		throw new Error("Executor network budget exceeded");
	const request = new Request(input, init);
	const headers = [...request.headers.entries()];
	const body = request.body
		? Buffer.from(await request.arrayBuffer()).toString("base64")
		: null;
	const response = await nativeFetch(proxy, {
		method: "POST",
		signal:
			request.method === "DELETE"
				? AbortSignal.timeout(1000)
				: AbortSignal.any([request.signal, state.signal]),
		headers: {
			authorization: `Bearer ${token}`,
			"content-type": "application/json",
		},
		body: JSON.stringify({
			url: request.url,
			method: request.method,
			headers,
			body,
		}),
	});
	// Proxy responses retain the upstream status/headers, including WWW-Authenticate.
	const reader = response.body?.getReader();
	const stream = reader
		? new ReadableStream<Uint8Array>({
				async pull(controller) {
					try {
						const next = await reader.read();
						if (next.done) {
							controller.close();
							return;
						}
						state.bytes += next.value.byteLength;
						if (state.bytes > MAX_BYTES) {
							await reader.cancel();
							controller.error(new Error("Executor response budget exceeded"));
							return;
						}
						controller.enqueue(next.value);
					} catch (error) {
						controller.error(error);
					}
				},
				cancel(reason) {
					return reader.cancel(reason);
				},
			})
		: null;
	return new Response(stream, {
		status: response.status,
		headers: response.headers,
	});
}) as typeof fetch;

const aad = (id: string) => Buffer.from(`zuse-executor-state:v1:${id}`);
const state: StateCodec = {
	seal(id, plaintext) {
		const iv = randomBytes(12),
			cipher = createCipheriv("aes-256-gcm", key, iv);
		cipher.setAAD(aad(id));
		const body = Buffer.concat([cipher.update(plaintext), cipher.final()]);
		return Buffer.concat([iv, cipher.getAuthTag(), body]).toString("base64");
	},
	open(id, sealed) {
		const envelope = Buffer.from(sealed, "base64");
		const cipher = createDecipheriv(
			"aes-256-gcm",
			key,
			envelope.subarray(0, 12),
		);
		cipher.setAAD(aad(id));
		cipher.setAuthTag(envelope.subarray(12, 28));
		return Buffer.concat([
			cipher.update(envelope.subarray(28)),
			cipher.final(),
		]).toString();
	},
};
/**
 * Definitions by tenant-qualified id (sha256 of owner and definition id), so
 * one tenant's id never names another tenant's definition. Each request sets
 * the definition Rust resolved for it before using it. Bounded; an evicted
 * definition is set again by the next request that needs it.
 */
const definitions = new Map<string, EnginePlugin>();
const MAX_DEFINITIONS = 1024;
const define = (id: string, plugin: EnginePlugin) => {
	definitions.delete(id);
	definitions.set(id, plugin);
	for (const oldest of definitions.keys()) {
		if (definitions.size <= MAX_DEFINITIONS) break;
		definitions.delete(oldest);
	}
};
const openEngine = (migrate = false) =>
	createPluginEngine({
		state,
		databaseUrl,
		maxConnections,
		encryptionKey,
		credentialScope: "zuse-native-postgres-v1",
		plugin: (id) => definitions.get(id),
		fetch: globalThis.fetch,
		requestScope: {
			capture: () => budget.getStore(),
			enter: (scope, work) =>
				scope === undefined ? work() : budget.run(scope as Budget, work),
		},
		migrate,
		allowLoopbackHttp: process.env.ZUSE_EXECUTOR_TEST_LOOPBACK === "1",
	});
/** One engine, and so one connection pool, for the life of the process. */
let shared: Promise<PluginEngine> | undefined;
const sharedEngine = () => {
	if (!shared) {
		const opening = openEngine();
		shared = opening;
		opening.catch(() => {
			if (shared === opening) shared = undefined;
		});
	}
	return shared;
};
if (process.argv.includes("--migrate")) {
	const migrated = await openEngine(true);
	await migrated.close();
	process.exit(0);
}

async function serveMcp(input: any) {
	const server = new Server(
		{ name: "Zuse", version: "2.0.0" },
		{ capabilities: { tools: {} } },
	);
	const transport = new WebStandardStreamableHTTPServerTransport({
		enableJsonResponse: true,
	});
	let command: unknown;
	server.setRequestHandler(ListToolsRequestSchema, async () => ({
		tools: tools as any,
	}));
	server.setRequestHandler(CallToolRequestSchema, async ({ params }) => {
		const action = (
			{
				plugins_list: "list",
				plugins_search: "search",
				plugins_schema: "schema",
				plugins_call: "call",
			} as Record<string, string>
		)[params.name];
		const args = params.arguments ?? {};
		if (
			!action ||
			(action === "search" && typeof args.query !== "string") ||
			(["schema", "call"].includes(action) &&
				typeof args.address !== "string") ||
			(action === "call" &&
				(!args.arguments ||
					typeof args.arguments !== "object" ||
					Array.isArray(args.arguments)))
		) {
			return {
				isError: true,
				content: [{ type: "text", text: "Invalid plugin request." }],
			};
		}
		command = { ...args, action };
		return input.failed
			? {
					isError: true,
					content: [
						{
							type: "text",
							text: "Plugin request failed. Check your connection in Zuse.",
						},
					],
				}
			: {
					content: [
						{ type: "text", text: JSON.stringify(input.result ?? null) },
					],
				};
	});
	await server.connect(transport);
	try {
		const request = new Request("https://plugins.invalid/mcp", {
			method: "POST",
			headers: input.headers,
			body: JSON.stringify(input.message),
		});
		const response = await transport.handleRequest(request, {
			parsedBody: input.message,
		});
		const text = await response.text();
		return {
			status: response.status,
			body: text ? JSON.parse(text) : null,
			command,
		};
	} finally {
		await server.close();
	}
}

async function dispatch(input: any) {
	if (input.operation === "mcp") return serveMcp(input);
	if (input.operation === "ready") {
		await (await sharedEngine()).ready();
		return { ready: true };
	}
	const owner = input.owner;
	if (typeof owner !== "string" || !owner) throw new Error("Invalid owner");
	// Registration uses a tenant-qualified opaque ID. The immutable definition
	// fingerprint is included in the Rust connection record and checked there.
	const definition: EnginePlugin = input.definition;
	if (definition) {
		const id = createHash("sha256")
			.update(JSON.stringify([owner, definition.id]))
			.digest("hex");
		define(id, { ...definition, id });
		input.plugin = id;
	}
	const ref: EngineConnection = input.ref;
	const engine = await sharedEngine();
	switch (input.operation) {
		case "prepare":
			return await engine.prepare(
				owner,
				input.subject,
				input.id,
				input.plugin,
				input.auth,
			);
		case "start":
			return await engine.start(owner, ref, input.label, input.redirectUri);
		case "submit":
			return await engine.submit(owner, ref, input.label, input.value);
		case "complete":
			return await engine.complete(owner, ref, input.callbackUrl);
		case "remove":
			return await engine.remove(owner, input.id, ref).then(() => null);
		case "list":
			return await engine.list(owner, ref);
		case "schema":
			return await engine.schema(owner, ref, input.tool);
		case "call":
			return await engine.call(owner, ref, input.tool, input.arguments);
		default:
			throw new Error("Unknown operation");
	}
}
let active = 0;
const server = createServer(async (req, res) => {
	const supplied = Buffer.from(req.headers.authorization ?? ""),
		expected = Buffer.from(`Bearer ${token}`);
	if (
		supplied.length !== expected.length ||
		!timingSafeEqual(supplied, expected) ||
		req.headers.origin ||
		req.method !== "POST"
	) {
		res.writeHead(403);
		res.end();
		return;
	}
	if (active >= 8) {
		res.writeHead(503);
		res.end();
		return;
	}
	active++;
	const abort = new AbortController();
	const timeout = setTimeout(() => abort.abort(), 60_000);
	timeout.unref();
	res.once("close", () => {
		if (!res.writableEnded) abort.abort();
	});
	const scope: Budget = {
		bytes: 0,
		requests: 0,
		signal: abort.signal,
		open: true,
	};
	try {
		let bytes = 0;
		const chunks: Buffer[] = [];
		for await (const chunk of req) {
			bytes += chunk.length;
			if (bytes > 256 * 1024) throw new Error("Request too large");
			chunks.push(chunk);
		}
		const input = JSON.parse(Buffer.concat(chunks).toString());
		(req as any).executorOperation = [
			"ready",
			"mcp",
			"prepare",
			"start",
			"submit",
			"complete",
			"remove",
			"list",
			"schema",
			"call",
		].includes(input.operation)
			? input.operation
			: "unknown";
		const value = await budget.run(scope, () => dispatch(input));
		const output = JSON.stringify({ ok: true, value });
		if (Buffer.byteLength(output) > MAX_BYTES)
			throw new Error("Response too large");
		res.writeHead(200, { "content-type": "application/json" });
		res.end(output);
	} catch (error: any) {
		const allowed = [
			"client_registration_unsupported",
			"auth_unsupported",
			"unreachable",
		];
		// Never serialize provider errors, SQL errors, credentials, URLs or stacks.
		const code =
			error?._tag === "OAuthReconnectRequired"
				? "needs_auth"
				: allowed.includes(error?.code)
					? error.code
					: "operation_failed";
		console.error(
			JSON.stringify({
				event: "executor_operation_failed",
				operation:
					typeof (req as any).executorOperation === "string"
						? (req as any).executorOperation
						: "unknown",
				tag: typeof error?._tag === "string" ? error._tag : "unknown",
				code,
			}),
		);
		res.writeHead(400, { "content-type": "application/json" });
		res.end(JSON.stringify({ ok: false, code }));
	} finally {
		scope.open = false;
		clearTimeout(timeout);
		active--;
	}
});
server.listen(0, "127.0.0.1", () => {
	console.log(JSON.stringify({ port: (server.address() as any).port }));
});
for (const signal of ["SIGTERM", "SIGINT"] as const)
	process.on(signal, async () => {
		server.close();
		await shared?.then((current) => current.close()).catch(() => undefined);
		process.exit(0);
	});
