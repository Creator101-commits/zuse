/** Cloud catalog refreshes run fast while a workspace changes and slow down while nothing does. */
export const CATALOG_TRANSITION_POLL_MS = 4_000;
export const CATALOG_IDLE_POLL_MS = 10_000;
export const CATALOG_IDLE_POLL_MAX_MS = 60_000;

/** Sign-in stays responsive while the user is likely finishing in the browser, then eases off. */
export const LOGIN_POLL_MS = 2_000;
export const LOGIN_POLL_SLOW_MS = 10_000;
export const LOGIN_POLL_FAST_WINDOW_MS = 60_000;

/** Doubling delay for the nth consecutive quiet poll, capped. */
export const backoffDelay = (
	attempt: number,
	initialMs: number,
	maxMs: number,
): number =>
	Math.min(maxMs, initialMs * 2 ** Math.min(Math.max(0, attempt), 16));

export const catalogPollDelay = (
	transitioning: boolean,
	idleAttempt: number,
): number =>
	transitioning
		? CATALOG_TRANSITION_POLL_MS
		: backoffDelay(idleAttempt, CATALOG_IDLE_POLL_MS, CATALOG_IDLE_POLL_MAX_MS);

export const loginPollDelay = (elapsedMs: number): number =>
	elapsedMs < LOGIN_POLL_FAST_WINDOW_MS ? LOGIN_POLL_MS : LOGIN_POLL_SLOW_MS;
