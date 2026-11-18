/**
 * Seals `zuse_executor_state` rows. The engine stores them in the same
 * PostgreSQL database as the Rust API, on its own connection pool.
 */
export interface StateCodec {
	/** Encrypts one row's JSON text, bound to its id. */
	seal(id: string, plaintext: string): string;
	/** Decrypts and authenticates one row; throws when it does not verify. */
	open(id: string, sealed: string): string;
}

/** `header` sends a saved key in the plugin's header on every request. */
export type EngineAuth = "none" | "oauth" | "header";

/** Plain values only: the upstream Effect runtime stays inside the bundled adapter. */
export interface EnginePlugin {
	readonly id: string;
	readonly name: string;
	readonly endpoint: string;
	/** OAuth discovery URL when it differs from the endpoint. */
	readonly oauthDiscovery?: string;
	readonly scopes?: readonly string[];
	/** Header carrying the key for `header` auth. */
	readonly header?: string;
}
export interface EngineOptions {
	readonly state: StateCodec;
	readonly databaseUrl: string;
	/** Database connections for the SDK and state rows together (default 1). */
	readonly maxConnections?: number;
	readonly migrate?: boolean;
	readonly allowLoopbackHttp?: boolean;
	readonly encryptionKey: string;
	readonly credentialScope: string;
	/**
	 * Server-owned catalog and workspace-added servers, by tenant-qualified id.
	 * Definitions are registered lazily per (definition, auth).
	 */
	readonly plugin: (id: string) => EnginePlugin | undefined;
	readonly fetch: typeof fetch;
	/**
	 * The calling request's network budget. `capture` runs when an operation
	 * starts; `enter` runs SDK network work inside the captured budget, since
	 * shared connections can resume work in another request's context.
	 */
	readonly requestScope: {
		capture(): unknown;
		enter<A>(scope: unknown, work: () => A): A;
	};
}
export interface EngineConnection {
	readonly app: string;
	readonly profile: string;
	readonly connection?: string;
	readonly account?: string;
	/** Catalog id and resolved auth; select the registered definition after restarts. */
	readonly plugin: string;
	readonly auth: EngineAuth;
}
/** Failures callers can explain to users. Everything else is an opaque failure. */
export type PluginEngineErrorCode =
	| "client_registration_unsupported"
	| "auth_unsupported"
	| "unreachable";
export interface PluginEngine {
	prepare(
		owner: string,
		subject: string,
		id: string,
		pluginId: string,
		auth: EngineAuth,
	): Promise<EngineConnection>;
	start(
		owner: string,
		ref: EngineConnection,
		label: string,
		redirectUri: string,
	): Promise<string>;
	/** Saves a key for a `header` connection; the caller verifies it first. */
	submit(
		owner: string,
		ref: EngineConnection,
		label: string,
		value: string,
	): Promise<EngineConnection>;
	complete(
		owner: string,
		ref: EngineConnection,
		callbackUrl: string,
	): Promise<EngineConnection>;
	remove(
		owner: string,
		id: string,
		ref?: Partial<EngineConnection>,
	): Promise<void>;
	list(
		owner: string,
		ref: EngineConnection,
	): Promise<readonly { name: string; description: string }[]>;
	schema(owner: string, ref: EngineConnection, tool: string): Promise<unknown>;
	call(
		owner: string,
		ref: EngineConnection,
		tool: string,
		input: unknown,
	): Promise<unknown>;
	/**
	 * Serve's database change epoch, sent with every request: `null` while
	 * serve cannot forward change notifications, so nothing is kept.
	 */
	observe(epoch: number | null): void;
	/** One cheap statement on the engine's pool. */
	ready(): Promise<void>;
	close(): Promise<void>;
}
