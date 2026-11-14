import { useAtomValue } from "@effect/atom-react";
import { useEffect, useRef } from "react";

import {
	captureMobileAnalytics,
	setMobileAnalyticsAccount,
	trackMobileScreen,
} from "~/lib/analytics";
import { mobileAnalyticsScreen } from "~/lib/mobile-analytics-screen";
import { analyticsHydratedAtom, hydrateAnalytics } from "~/store/analytics";
import { authAccountAtom, authHydratedAtom, hydrateAuth } from "~/store/auth";

export const useMobileAnalytics = (pathname: string): void => {
	const previousPath = useRef<string | null>(null);
	const account = useAtomValue(authAccountAtom);
	const accountId = account?.id ?? null;
	const authHydrated = useAtomValue(authHydratedAtom);
	const analyticsHydrated = useAtomValue(analyticsHydratedAtom);

	useEffect(() => {
		if (!authHydrated) void hydrateAuth();
	}, [authHydrated]);

	useEffect(() => {
		if (authHydrated && !analyticsHydrated) void hydrateAnalytics();
	}, [analyticsHydrated, authHydrated]);

	useEffect(() => {
		if (analyticsHydrated) void setMobileAnalyticsAccount(accountId);
	}, [accountId, analyticsHydrated]);

	useEffect(() => {
		if (analyticsHydrated) {
			const screen = mobileAnalyticsScreen(pathname);
			trackMobileScreen(screen);
			if (screen === "session" && previousPath.current !== pathname) {
				captureMobileAnalytics("chat opened");
			}
			previousPath.current = pathname;
		}
	}, [analyticsHydrated, pathname]);
};
