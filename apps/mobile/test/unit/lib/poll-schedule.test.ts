import { expect, test } from "vitest";

import {
	backoffDelay,
	catalogPollDelay,
	loginPollDelay,
} from "../../../src/lib/poll-schedule";

test("doubles quiet polls up to the cap", () => {
	expect(backoffDelay(0, 10_000, 60_000)).toBe(10_000);
	expect(backoffDelay(1, 10_000, 60_000)).toBe(20_000);
	expect(backoffDelay(2, 10_000, 60_000)).toBe(40_000);
	expect(backoffDelay(3, 10_000, 60_000)).toBe(60_000);
	expect(backoffDelay(1_000, 10_000, 60_000)).toBe(60_000);
	expect(backoffDelay(-1, 10_000, 60_000)).toBe(10_000);
});

test("catalog polls quickly only while a workspace is changing", () => {
	expect(catalogPollDelay(true, 5)).toBe(4_000);
	expect(catalogPollDelay(false, 0)).toBe(10_000);
	expect(catalogPollDelay(false, 9)).toBe(60_000);
});

test("sign-in stays responsive for a minute, then eases off", () => {
	expect(loginPollDelay(0)).toBe(2_000);
	expect(loginPollDelay(59_999)).toBe(2_000);
	expect(loginPollDelay(60_000)).toBe(10_000);
	expect(loginPollDelay(600_000)).toBe(10_000);
});
