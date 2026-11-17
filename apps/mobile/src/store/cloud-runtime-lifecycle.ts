import { useAtomValue } from "@effect/atom-react";
import { useEffect } from "react";
import { AppState } from "react-native";
import { cloudLifecycle } from "~/lib/cloud-lifecycle";
import { catalogPollDelay } from "~/lib/poll-schedule";
import { resetCloudRuntime } from "~/rpc/cloud-runtime";
import { disposeConnection } from "~/rpc/connection";
import { authAccountAtom } from "./auth";
import {
	cloudCatalogAtom,
	cloudConnectionsAtom,
	onCloudCatalogUserRefresh,
	refreshCloudCatalog,
	setCloudCatalogAccount,
} from "./cloud-catalog";
import { resetMessagesRuntime } from "./messages";
import {
	mobileClientBus,
	registerMobileEnvironment,
} from "./mobile-client-bus";
import { appAtomRegistry } from "./registry";
import { resetSessionsRuntime } from "./sessions";

let accountTeardown: Promise<unknown> = Promise.resolve();

/** Account catalog ownership is independent of any screen or paired device. */
export function useCloudRuntimeLifecycle(): void {
	const account = useAtomValue(authAccountAtom);
	const accountId = account?.id ?? null;
	const connections = useAtomValue(cloudConnectionsAtom);
	const catalog = useAtomValue(cloudCatalogAtom);
	useEffect(() => {
		setCloudCatalogAccount(accountId);
		if (accountId === null) return;
		let active = AppState.currentState !== "background";
		let disposed = false;
		let idleAttempt = 0;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const transitioning = () =>
			appAtomRegistry.get(cloudCatalogAtom).chats.some((summary) => {
				const lifecycle = cloudLifecycle(summary);
				return lifecycle === "starting" || lifecycle === "resuming";
			});
		// Fast while a workspace changes; idle polls back off until something happens.
		const schedule = () => {
			clearTimeout(timer);
			if (disposed || !active) return;
			timer = setTimeout(
				() => {
					idleAttempt = transitioning() ? 0 : idleAttempt + 1;
					void refreshCloudCatalog({ background: true }).then(
						schedule,
						schedule,
					);
				},
				catalogPollDelay(transitioning(), idleAttempt),
			);
		};
		const refreshNow = () => {
			idleAttempt = 0;
			if (!active) return;
			void refreshCloudCatalog({ background: true });
			schedule();
		};
		refreshNow();

		const removeUserRefresh = onCloudCatalogUserRefresh(() => {
			idleAttempt = 0;
			schedule();
		});
		// A wake started elsewhere (for example by sending a message) should not wait out an idle delay.
		let wasTransitioning = transitioning();
		const removeCatalog = appAtomRegistry.subscribe(cloudCatalogAtom, () => {
			const changing = transitioning();
			if (changing && !wasTransitioning) {
				idleAttempt = 0;
				schedule();
			}
			wasTransitioning = changing;
		});
		const subscription = AppState.addEventListener("change", (state) => {
			const wasActive = active;
			active = state === "active";
			if (!active) clearTimeout(timer);
			else if (!wasActive) refreshNow();
		});
		return () => {
			disposed = true;
			clearTimeout(timer);
			removeUserRefresh();
			removeCatalog();
			subscription.remove();
			const previousConnections = appAtomRegistry.get(cloudConnectionsAtom);
			setCloudCatalogAccount(null);
			resetCloudRuntime();
			accountTeardown = Promise.allSettled([
				...previousConnections.map(disposeConnection),
				resetMessagesRuntime(),
				resetSessionsRuntime(),
			]);
		};
	}, [accountId]);
	useEffect(() => {
		if (account?.id !== catalog.accountId) return;
		let active = true;
		void accountTeardown.then(() => {
			if (
				!active ||
				appAtomRegistry.get(cloudCatalogAtom).accountId !== account?.id
			)
				return;
			for (const connection of connections) {
				const environmentId = registerMobileEnvironment(
					connection.key,
					connection,
				);
				void mobileClientBus()
					.flushDurableOutbox(environmentId)
					.catch(() => undefined);
			}
		});
		return () => {
			active = false;
		};
	}, [account?.id, catalog.accountId, connections]);
}
